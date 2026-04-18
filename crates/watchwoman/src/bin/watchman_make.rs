//! watchman-make — re-run a command when matching files change.
//!
//! A thin wrapper around `subscribe` that debounces events and runs a
//! command when the dust settles.  Upstream `watchman-make` has a
//! richer CLI (targets, settle time per target); we ship the common
//! case and document the rest as a follow-up.

use std::io::{BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use clap::Parser;
use indexmap::IndexMap;
use watchwoman_protocol::{json, Value};

#[derive(Debug, Parser)]
#[command(
    name = "watchman-make",
    about = "Re-run a command whenever matching files change (debounced)."
)]
struct Cli {
    /// Glob pattern(s) of files to watch.  Repeat for multiple.
    #[arg(short = 'p', long = "pattern")]
    patterns: Vec<String>,

    /// Settle period in seconds — wait this long after the last
    /// change before running the command.  Rapid-fire writes coalesce.
    #[arg(short = 's', long = "settle", default_value_t = 1.0)]
    settle_secs: f64,

    /// Run this command on each fire. Everything after `--` is passed
    /// through verbatim.
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,

    /// Override the socket path.
    #[arg(long, env = "WATCHMAN_SOCK")]
    sockname: Option<String>,

    /// Path to watch.  Defaults to the current working directory.
    #[arg(long, short = 'r', long = "root")]
    root: Option<PathBuf>,
}

fn main() -> ExitCode {
    if let Err(e) = run() {
        eprintln!("watchman-make: {e:#}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.command.is_empty() {
        anyhow::bail!(
            "watchman-make: missing command — pass it after `--`, e.g. `watchman-make -p '*.rs' -- cargo test`"
        );
    }
    let sock_path = watchwoman::sock::resolve(cli.sockname.as_deref())?;
    let root = cli
        .root
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"));
    let root = std::fs::canonicalize(&root).unwrap_or(root);

    let mut stream = UnixStream::connect(&sock_path)
        .with_context(|| format!("connecting to {}", sock_path.display()))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    // Establish the watch.
    send_pdu(
        &mut stream,
        &Value::Array(vec![
            Value::String("watch-project".into()),
            Value::String(root.to_string_lossy().into()),
        ]),
    )?;
    let _ = read_pdu(&mut stream)?;

    // Subscribe.
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
    send_pdu(
        &mut stream,
        &Value::Array(vec![
            Value::String("subscribe".into()),
            Value::String(root.to_string_lossy().into()),
            Value::String(format!("make-{}", std::process::id())),
            Value::Object(sub),
        ]),
    )?;
    let _ = read_pdu(&mut stream)?;

    let settle = Duration::from_secs_f64(cli.settle_secs.max(0.0));
    let mut last_event: Option<Instant> = None;
    stream.set_read_timeout(Some(settle))?;

    loop {
        match read_pdu(&mut stream) {
            Ok(_pdu) => {
                last_event = Some(Instant::now());
            }
            Err(e) => {
                let timed_out = e.downcast_ref::<std::io::Error>().is_some_and(|io| {
                    matches!(
                        io.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    )
                });
                if !timed_out {
                    return Err(e);
                }
            }
        }

        if let Some(t) = last_event {
            if t.elapsed() >= settle {
                last_event = None;
                if let Err(err) = fire(&root, &cli.command) {
                    eprintln!("watchman-make: command failed: {err:#}");
                }
            }
        }
    }
}

fn fire(cwd: &std::path::Path, argv: &[String]) -> anyhow::Result<()> {
    let (exe, rest) = argv.split_first().unwrap();
    let status = Command::new(exe)
        .args(rest)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("spawning {exe}"))?;
    if !status.success() {
        eprintln!(
            "watchman-make: `{}` exited with {:?}",
            argv.join(" "),
            status.code()
        );
    }
    Ok(())
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
