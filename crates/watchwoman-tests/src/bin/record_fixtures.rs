//! Record watchman responses as golden fixtures.
//!
//! Spawns the real `watchman` binary in an isolated state dir, runs a
//! catalog of commands, and writes each response to
//! `crates/watchwoman-protocol/tests/fixtures/` in both JSON and BSER
//! v2 encodings.  The fixtures are then round-tripped in unit tests
//! to prove our codec accepts real watchman output verbatim.
//!
//! Usage:
//!
//! ```sh
//! cargo run -p watchwoman-tests --bin record-fixtures
//! ```
//!
//! Requires `watchman` on `$PATH`.  Skips cleanly if not present.

use std::fs;
use std::io::{BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use watchwoman_protocol::{bser, json, Encoding, Value};
use watchwoman_tests::{Harness, Scratch, TargetBinary};

struct Scenario {
    name: &'static str,
    build_pdu: fn(&Path) -> Value,
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "version",
        build_pdu: |_root| Value::Array(vec![Value::String("version".into())]),
    },
    Scenario {
        name: "list_capabilities",
        build_pdu: |_root| Value::Array(vec![Value::String("list-capabilities".into())]),
    },
    Scenario {
        name: "get_sockname",
        build_pdu: |_root| Value::Array(vec![Value::String("get-sockname".into())]),
    },
    Scenario {
        name: "watch_project",
        build_pdu: |root| {
            Value::Array(vec![
                Value::String("watch-project".into()),
                Value::String(root.to_string_lossy().into()),
            ])
        },
    },
    Scenario {
        name: "watch_list",
        build_pdu: |_root| Value::Array(vec![Value::String("watch-list".into())]),
    },
    Scenario {
        name: "clock",
        build_pdu: |root| {
            Value::Array(vec![
                Value::String("clock".into()),
                Value::String(root.to_string_lossy().into()),
            ])
        },
    },
    Scenario {
        name: "query_suffix",
        build_pdu: |root| {
            let mut spec = indexmap::IndexMap::new();
            spec.insert(
                "suffix".to_owned(),
                Value::Array(vec![Value::String("rs".into())]),
            );
            spec.insert(
                "fields".to_owned(),
                Value::Array(vec![Value::String("name".into())]),
            );
            Value::Array(vec![
                Value::String("query".into()),
                Value::String(root.to_string_lossy().into()),
                Value::Object(spec),
            ])
        },
    },
];

fn main() -> anyhow::Result<()> {
    let out_dir = fixtures_dir();
    fs::create_dir_all(&out_dir)?;

    let harness = match Harness::spawn_with(TargetBinary::Watchman) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("note: `watchman` unavailable ({e:#}); skipping recorder");
            return Ok(());
        }
    };
    let scratch = Scratch::new()?;
    fs::write(scratch.path().join("main.rs"), "fn main() {}")?;
    fs::write(scratch.path().join("lib.rs"), "pub fn noop() {}")?;
    fs::write(scratch.path().join("notes.md"), "hi")?;

    // Seed a watch-project so watch-list / clock / query can respond.
    {
        let mut s = connect(harness.sock())?;
        send_json(
            &mut s,
            &Value::Array(vec![
                Value::String("watch-project".into()),
                Value::String(scratch.path().to_string_lossy().into()),
            ]),
        )?;
        let _ = read_json(&mut s)?;
    }

    for scenario in SCENARIOS {
        let pdu = (scenario.build_pdu)(scratch.path());

        // JSON fixture.
        {
            let mut s = connect(harness.sock())?;
            send_json(&mut s, &pdu)?;
            let resp = read_json(&mut s)?;
            let path = out_dir.join(format!("{}.json", scenario.name));
            fs::write(&path, serde_json::to_vec_pretty(&value_to_serde(&resp))?)?;
            println!("recorded {}", path.display());
        }

        // BSER v2 fixture.
        {
            let mut s = connect(harness.sock())?;
            send_bser(&mut s, &pdu, Encoding::BserV2)?;
            let (resp, _) = read_bser(&mut s)?;
            let path = out_dir.join(format!("{}.bser2", scenario.name));
            let bytes = bser::encode_pdu(&resp, Encoding::BserV2)?;
            fs::write(&path, &bytes)?;
            println!("recorded {}", path.display());
        }
    }

    Ok(())
}

fn connect(sock: &Path) -> anyhow::Result<UnixStream> {
    let s = UnixStream::connect(sock)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    s.set_write_timeout(Some(Duration::from_secs(10)))?;
    Ok(s)
}

fn send_json(stream: &mut UnixStream, pdu: &Value) -> anyhow::Result<()> {
    json::encode_pdu(stream, pdu)?;
    stream.flush()?;
    Ok(())
}

fn read_json(stream: &mut UnixStream) -> anyhow::Result<Value> {
    let mut reader = BufReader::new(stream);
    json::read_pdu(&mut reader)?.ok_or_else(|| anyhow::anyhow!("no JSON response"))
}

fn send_bser(stream: &mut UnixStream, pdu: &Value, enc: Encoding) -> anyhow::Result<()> {
    let bytes = bser::encode_pdu(pdu, enc)?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

fn read_bser(stream: &mut UnixStream) -> anyhow::Result<(Value, Encoding)> {
    let mut reader = BufReader::new(stream);
    bser::read_pdu(&mut reader)?.ok_or_else(|| anyhow::anyhow!("no BSER response"))
}

fn fixtures_dir() -> PathBuf {
    // This binary lives in crates/watchwoman-tests/src/bin; fixtures go
    // up two directories into crates/watchwoman-protocol/tests/fixtures.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo env");
    PathBuf::from(manifest_dir)
        .parent()
        .expect("crates/")
        .join("watchwoman-protocol")
        .join("tests")
        .join("fixtures")
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
