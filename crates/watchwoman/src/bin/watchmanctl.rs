//! watchmanctl — thin control wrapper around the daemon.
//!
//! Just enough of upstream's `watchmanctl` for operators to script
//! against: shutdown, status (`watch-list`), log-level, and
//! `debug-recrawl` on every watched root.

use std::io::{BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use watchwoman_protocol::{json, Value};

#[derive(Debug, Parser)]
#[command(
    name = "watchmanctl",
    about = "Control the watchwoman daemon: status, shutdown, log-level, recrawl."
)]
struct Cli {
    #[arg(long, env = "WATCHMAN_SOCK")]
    sockname: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Print watched roots.
    Status,
    /// Tear the daemon down.  Next CLI call will auto-spawn a fresh one.
    Shutdown,
    /// Set or read the server log level (debug/info/warn/error).
    LogLevel { level: Option<String> },
    /// Force a full rescan of every watched root.
    Recrawl,
}

fn main() -> ExitCode {
    if let Err(e) = run() {
        eprintln!("watchmanctl: {e:#}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let sock = watchwoman::sock::resolve(cli.sockname.as_deref())?;

    match cli.cmd {
        Cmd::Status => {
            print(&call(&sock, &[Value::String("watch-list".into())])?);
        }
        Cmd::Shutdown => {
            print(&call(&sock, &[Value::String("shutdown-server".into())])?);
        }
        Cmd::LogLevel { level } => {
            let mut argv = vec![Value::String("log-level".into())];
            if let Some(l) = level {
                argv.push(Value::String(l));
            }
            print(&call(&sock, &argv)?);
        }
        Cmd::Recrawl => {
            let list = call(&sock, &[Value::String("watch-list".into())])?;
            let roots: Vec<Value> = list
                .as_object()
                .and_then(|o| o.get("roots"))
                .and_then(Value::as_array)
                .map(<[Value]>::to_vec)
                .unwrap_or_default();
            for root in roots {
                let argv = vec![Value::String("debug-recrawl".into()), root.clone()];
                match call(&sock, &argv) {
                    Ok(v) => print(&v),
                    Err(e) => eprintln!("watchmanctl: recrawl of {root:?} failed: {e:#}"),
                }
            }
        }
    }
    Ok(())
}

fn call(sock: &Path, argv: &[Value]) -> anyhow::Result<Value> {
    let mut stream =
        UnixStream::connect(sock).with_context(|| format!("connecting to {}", sock.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let pdu = Value::Array(argv.to_vec());
    json::encode_pdu(&mut stream, &pdu)?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    json::read_pdu(&mut reader)?.context("daemon closed connection")
}

fn print(v: &Value) {
    let j = value_to_serde(v);
    if let Ok(s) = serde_json::to_string_pretty(&j) {
        println!("{s}");
    }
}

fn value_to_serde(v: &Value) -> serde_json::Value {
    use serde_json::{Number, Value as J};
    match v {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => J::Number(Number::from(*i)),
        Value::Real(f) => Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
        Value::String(s) => J::String(s.clone()),
        Value::Bytes(b) => J::String(String::from_utf8_lossy(b).into_owned()),
        Value::Array(a) => J::Array(a.iter().map(value_to_serde).collect()),
        Value::Object(o) => {
            let mut m = serde_json::Map::new();
            for (k, val) in o {
                m.insert(k.clone(), value_to_serde(val));
            }
            J::Object(m)
        }
        Value::Template { keys, rows } => {
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let mut m = serde_json::Map::new();
                for (k, val) in keys.iter().zip(row.iter()) {
                    m.insert(k.clone(), value_to_serde(val));
                }
                out.push(J::Object(m));
            }
            J::Array(out)
        }
    }
}
