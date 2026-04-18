//! watchman-wait — block until a matching file changes, print names.
//!
//! A thin wrapper around `subscribe` that speaks directly to the
//! watchwoman daemon on its unix socket.  Mirrors the CLI surface of
//! upstream `watchman-wait` closely enough for drop-in use by shell
//! scripts: `watchman-wait [-p pat] [-t ms] [-0] [path...]`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use clap::Parser;
use indexmap::IndexMap;
use watchwoman_protocol::{json, Value};

#[derive(Debug, Parser)]
#[command(
    name = "watchman-wait",
    about = "Block until one or more files change, then print their names."
)]
struct Cli {
    /// One or more glob patterns to filter changes.  Matches basename.
    /// Repeat the flag for multiple patterns.
    #[arg(short = 'p', long = "pattern")]
    patterns: Vec<String>,

    /// Exit after this many seconds with no matches.  0 = wait forever.
    #[arg(short = 't', long = "timeout", default_value_t = 0)]
    timeout_secs: u64,

    /// Exit after the first N matching files.  0 = loop forever.
    #[arg(short = 'm', long = "max-events", default_value_t = 0)]
    max_events: usize,

    /// Separate filenames with NUL instead of newline (for `xargs -0`).
    #[arg(short = '0', long = "null")]
    null: bool,

    /// Override the socket path.  Falls back to $WATCHMAN_SOCK, then
    /// the watchwoman default.
    #[arg(long, env = "WATCHMAN_SOCK")]
    sockname: Option<String>,

    /// Watch these paths.  Defaults to the current working directory.
    paths: Vec<PathBuf>,
}

fn main() -> ExitCode {
    if let Err(e) = run() {
        eprintln!("watchman-wait: {e:#}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let sock_path = watchwoman::sock::resolve(cli.sockname.as_deref())?;

    let mut paths = cli.paths.clone();
    if paths.is_empty() {
        paths.push(std::env::current_dir()?);
    }

    let mut stream = connect(&sock_path)?;

    // watch-project each path so the daemon establishes a tree.
    for p in &paths {
        let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        send_pdu(
            &mut stream,
            &Value::Array(vec![
                Value::String("watch-project".into()),
                Value::String(abs.to_string_lossy().into()),
            ]),
        )?;
        let _ = read_pdu(&mut stream)?; // swallow response
    }

    // Subscribe on the first root.  Multi-root is possible but rare
    // for `watchman-wait`; callers that need it can run separate
    // processes.  Matches upstream behaviour.
    let root = std::fs::canonicalize(&paths[0]).unwrap_or_else(|_| paths[0].clone());
    let mut sub = IndexMap::new();
    sub.insert(
        "fields".into(),
        Value::Array(vec![Value::String("name".into())]),
    );
    sub.insert("empty_on_fresh_instance".into(), Value::Bool(true));
    if !cli.patterns.is_empty() {
        let mut any = Vec::with_capacity(cli.patterns.len());
        for pat in &cli.patterns {
            any.push(Value::Array(vec![
                Value::String("match".into()),
                Value::String(pat.clone()),
                Value::String("basename".into()),
            ]));
        }
        let expr = if any.len() == 1 {
            any.into_iter().next().unwrap()
        } else {
            let mut arr = vec![Value::String("anyof".into())];
            arr.extend(any);
            Value::Array(arr)
        };
        sub.insert("expression".into(), expr);
    }

    let sub_name = format!("wait-{}", std::process::id());
    send_pdu(
        &mut stream,
        &Value::Array(vec![
            Value::String("subscribe".into()),
            Value::String(root.to_string_lossy().into()),
            Value::String(sub_name.clone()),
            Value::Object(sub),
        ]),
    )?;

    // Swallow the initial subscribe-confirmation PDU.
    let _ = read_pdu(&mut stream)?;

    let deadline = if cli.timeout_secs > 0 {
        Some(Instant::now() + Duration::from_secs(cli.timeout_secs))
    } else {
        None
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let sep: u8 = if cli.null { 0 } else { b'\n' };
    let mut emitted: usize = 0;

    loop {
        if let Some(d) = deadline {
            let remaining = d.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            stream.set_read_timeout(Some(remaining))?;
        }

        match read_pdu(&mut stream) {
            Ok(pdu) => {
                if let Some(files) = pdu
                    .as_object()
                    .and_then(|o| o.get("files"))
                    .and_then(Value::as_array)
                {
                    for f in files {
                        let name = match f {
                            Value::String(s) => s.clone(),
                            Value::Object(o) => o
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_owned(),
                            _ => continue,
                        };
                        if name.is_empty() {
                            continue;
                        }
                        out.write_all(name.as_bytes())?;
                        out.write_all(&[sep])?;
                        out.flush()?;
                        emitted += 1;
                        if cli.max_events > 0 && emitted >= cli.max_events {
                            return Ok(());
                        }
                    }
                }
            }
            Err(e) => {
                if let Some(io) = e.downcast_ref::<std::io::Error>() {
                    if matches!(
                        io.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) {
                        // Timeout reached; graceful exit.
                        return Ok(());
                    }
                }
                return Err(e);
            }
        }
    }
}

fn connect(sock: &std::path::Path) -> anyhow::Result<UnixStream> {
    let s = UnixStream::connect(sock)
        .with_context(|| format!("connecting to {} — is the daemon running?", sock.display()))?;
    s.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(s)
}

fn send_pdu(s: &mut UnixStream, v: &Value) -> anyhow::Result<()> {
    json::encode_pdu(s, v)?;
    s.flush()?;
    Ok(())
}

fn read_pdu(s: &mut UnixStream) -> anyhow::Result<Value> {
    let mut r = BufReader::new(s.try_clone()?);
    json::read_pdu(&mut r)?.context("daemon closed connection")
}

#[allow(dead_code)]
fn _bufread_import(_: &dyn BufRead) {}
