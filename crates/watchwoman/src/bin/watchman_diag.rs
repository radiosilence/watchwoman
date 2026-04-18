//! watchman-diag — dump daemon state for bug reports.
//!
//! Upstream's `watchman-diag` shells out to the daemon for a grab-bag
//! of commands and prints them all in one document.  We do the same:
//! version, capabilities, sockname, pid, watched roots, plus named
//! cursors per root.  Pipe it into a gist when opening an issue.

use std::io::{BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser;
use indexmap::IndexMap;
use watchwoman_protocol::{json, Value};

#[derive(Debug, Parser)]
#[command(
    name = "watchman-diag",
    about = "Print daemon state for bug reports: version, caps, sockname, roots, cursors."
)]
struct Cli {
    #[arg(long, env = "WATCHMAN_SOCK")]
    sockname: Option<String>,
}

fn main() -> ExitCode {
    if let Err(e) = run() {
        eprintln!("watchman-diag: {e:#}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let sock = watchwoman::sock::resolve(cli.sockname.as_deref())?;

    let mut report: IndexMap<String, Value> = IndexMap::new();
    report.insert("sockname".into(), call(&sock, &["get-sockname"])?);
    report.insert("version".into(), call(&sock, &["version"])?);
    report.insert("capabilities".into(), call(&sock, &["list-capabilities"])?);
    report.insert("pid".into(), call(&sock, &["get-pid"])?);
    let roots = call(&sock, &["watch-list"])?;
    report.insert("roots".into(), roots.clone());

    if let Some(list) = roots
        .as_object()
        .and_then(|o| o.get("roots"))
        .and_then(Value::as_array)
    {
        let mut per_root = IndexMap::new();
        for entry in list {
            let Some(p) = entry.as_str() else { continue };
            let mut root_report = IndexMap::new();
            if let Ok(cursors) = call(&sock, &["debug-show-cursors", p]) {
                root_report.insert("cursors".into(), cursors);
            }
            if let Ok(cfg) = call(&sock, &["get-config", p]) {
                root_report.insert("config".into(), cfg);
            }
            if let Ok(triggers) = call(&sock, &["trigger-list", p]) {
                root_report.insert("triggers".into(), triggers);
            }
            per_root.insert(p.to_owned(), Value::Object(root_report));
        }
        report.insert("per_root".into(), Value::Object(per_root));
    }

    let out = serde_json::to_string_pretty(&value_to_serde(&Value::Object(report)))?;
    println!("{out}");
    Ok(())
}

fn call(sock: &Path, argv: &[&str]) -> anyhow::Result<Value> {
    let mut stream =
        UnixStream::connect(sock).with_context(|| format!("connecting to {}", sock.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let pdu = Value::Array(
        argv.iter()
            .map(|s| Value::String((*s).to_owned()))
            .collect(),
    );
    json::encode_pdu(&mut stream, &pdu)?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    json::read_pdu(&mut reader)?.context("daemon closed connection")
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
