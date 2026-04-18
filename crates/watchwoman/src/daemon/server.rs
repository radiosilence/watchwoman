//! Unix-socket accept loop + per-connection JSON dispatch.
//!
//! BSER support lives behind the encoding sniffer in the protocol crate;
//! the server will pick up BSER paths once that's implemented. For now
//! we only accept JSON PDUs which is what every CLI caller uses anyway.

use std::sync::Arc;

use anyhow::Context;
use indexmap::IndexMap;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use watchwoman_protocol::Value;

use super::state::DaemonState;
use crate::commands;

pub async fn serve(state: Arc<DaemonState>) -> anyhow::Result<()> {
    if let Some(parent) = state.sock_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // Best-effort cleanup of a stale socket left by a previous run.
    let _ = std::fs::remove_file(&state.sock_path);

    let listener = UnixListener::bind(&state.sock_path)
        .with_context(|| format!("binding {}", state.sock_path.display()))?;

    tracing::info!(path = ?state.sock_path, "watchwoman listening");

    loop {
        tokio::select! {
            accept = listener.accept() => match accept {
                Ok((stream, _)) => {
                    let state = state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, state).await {
                            tracing::debug!(?e, "connection ended");
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!(?e, "accept failed");
                }
            },
            _ = state.shutdown.notified() => break,
        }
        if state.is_shutting_down() {
            break;
        }
    }

    let _ = std::fs::remove_file(&state.sock_path);
    Ok(())
}

async fn handle_connection(stream: UnixStream, state: Arc<DaemonState>) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    loop {
        let mut line = Vec::with_capacity(256);
        let n = reader.read_until(b'\n', &mut line).await?;
        if n == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.is_empty() {
            continue;
        }

        let pdu: serde_json::Value = match serde_json::from_slice(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = error_response(format!("invalid JSON PDU: {e}"));
                write_pdu(&mut write_half, &err).await?;
                continue;
            }
        };
        let pdu = json_to_value(pdu);

        let state2 = state.clone();
        let response =
            match tokio::task::spawn_blocking(move || commands::dispatch(&state2, pdu)).await {
                Ok(v) => v,
                Err(join_err) => error_response(format!("dispatcher panicked: {join_err}")),
            };

        write_pdu(&mut write_half, &response).await?;

        if state.is_shutting_down() {
            break;
        }
    }
    Ok(())
}

async fn write_pdu(w: &mut tokio::net::unix::OwnedWriteHalf, value: &Value) -> anyhow::Result<()> {
    let mut buf = Vec::with_capacity(256);
    watchwoman_protocol::json::encode_pdu(&mut buf, value)?;
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

fn json_to_value(v: serde_json::Value) -> Value {
    use serde_json::Value as J;
    match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Real(f)
            } else {
                Value::Null
            }
        }
        J::String(s) => Value::String(s),
        J::Array(a) => Value::Array(a.into_iter().map(json_to_value).collect()),
        J::Object(o) => {
            let mut m = IndexMap::with_capacity(o.len());
            for (k, val) in o {
                m.insert(k, json_to_value(val));
            }
            Value::Object(m)
        }
    }
}

fn error_response(msg: String) -> Value {
    let mut m = IndexMap::new();
    m.insert("error".to_owned(), Value::String(msg));
    Value::Object(m)
}
