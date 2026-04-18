//! Unix-socket accept loop + per-connection I/O.
//!
//! Each connection:
//!   1. Sniffs the first byte to pick JSON vs BSER v1 vs BSER v2.
//!   2. Splits into a reader task (parse PDU → dispatch → enqueue
//!      response) and a writer task (drain mpsc → encode → write).
//!   3. Exposes a [`Session`] handle that command handlers clone when
//!      they need to push unilateral PDUs (subscriptions).

use std::sync::Arc;

use anyhow::Context;
use indexmap::IndexMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{unix::OwnedWriteHalf, UnixListener, UnixStream};
use tokio::sync::mpsc;
use watchwoman_protocol::{bser, json, Encoding, Value};

use super::session::Session;
use super::state::DaemonState;
use crate::commands;

pub async fn serve(state: Arc<DaemonState>) -> anyhow::Result<()> {
    if let Some(parent) = state.sock_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
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
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    // Sniff encoding off the first PDU.
    let (first_pdu, encoding) = match read_first_pdu(&mut reader).await? {
        Some(p) => p,
        None => return Ok(()),
    };

    let (tx, rx) = mpsc::unbounded_channel::<Value>();
    let session = Session::new(encoding, tx);

    let writer_task = tokio::spawn(writer_loop(write_half, rx, encoding));

    // Dispatch the already-read first PDU, then loop for subsequent ones.
    dispatch_and_send(&state, &session, first_pdu).await;

    loop {
        if session.is_closed() || state.is_shutting_down() {
            break;
        }
        let pdu = match read_pdu(&mut reader, encoding).await {
            Ok(Some(v)) => v,
            Ok(None) => break,
            Err(e) => {
                tracing::debug!(?e, "parse error, closing");
                let _ = session.send(error_response(format!("parse error: {e:#}")));
                break;
            }
        };
        dispatch_and_send(&state, &session, pdu).await;
    }

    drop(session);
    let _ = writer_task.await;
    Ok(())
}

async fn dispatch_and_send(state: &Arc<DaemonState>, session: &Session, pdu: Value) {
    let state_clone = state.clone();
    let session_clone = session.clone();
    let response = match tokio::task::spawn_blocking(move || {
        commands::dispatch(&state_clone, &session_clone, pdu)
    })
    .await
    {
        Ok(v) => v,
        Err(join_err) => error_response(format!("dispatcher panicked: {join_err}")),
    };
    let _ = session.send(response);
}

async fn writer_loop(
    mut write_half: OwnedWriteHalf,
    mut rx: mpsc::UnboundedReceiver<Value>,
    encoding: Encoding,
) {
    while let Some(value) = rx.recv().await {
        if let Err(e) = write_value(&mut write_half, &value, encoding).await {
            tracing::debug!(?e, "write failed, dropping connection");
            break;
        }
    }
}

async fn write_value(
    w: &mut OwnedWriteHalf,
    value: &Value,
    encoding: Encoding,
) -> anyhow::Result<()> {
    let mut buf = Vec::with_capacity(256);
    match encoding {
        Encoding::Json => {
            json::encode_pdu(&mut buf, value)?;
        }
        Encoding::BserV1 | Encoding::BserV2 => {
            buf = bser::encode_pdu(value, encoding)?;
        }
    }
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

async fn read_first_pdu<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> anyhow::Result<Option<(Value, Encoding)>> {
    // Peek the first byte to sniff encoding.
    let first = match read_u8(reader).await? {
        Some(b) => b,
        None => return Ok(None),
    };

    if first == 0x00 {
        let second = read_u8(reader)
            .await?
            .ok_or_else(|| anyhow::anyhow!("EOF after BSER magic byte"))?;
        let encoding = match second {
            0x01 => Encoding::BserV1,
            0x02 => {
                let mut caps = [0u8; 4];
                tokio::io::AsyncReadExt::read_exact(reader, &mut caps).await?;
                Encoding::BserV2
            }
            other => anyhow::bail!("unknown BSER version byte {other:#x}"),
        };
        let length = read_bser_int(reader).await?;
        let mut payload = vec![0u8; length as usize];
        tokio::io::AsyncReadExt::read_exact(reader, &mut payload).await?;
        let (value, _) = bser_decode_value(&payload)?;
        Ok(Some((value, encoding)))
    } else {
        // JSON line: the `first` byte is already consumed.  Concatenate
        // with the rest of the line up to `\n`.
        let mut buf = vec![first];
        tokio::io::AsyncBufReadExt::read_until(reader, b'\n', &mut buf).await?;
        if buf.last() == Some(&b'\n') {
            buf.pop();
        }
        if buf.is_empty() {
            return Err(anyhow::anyhow!("empty JSON PDU"));
        }
        let v: serde_json::Value = serde_json::from_slice(&buf)?;
        Ok(Some((json_to_value(v), Encoding::Json)))
    }
}

async fn read_pdu<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    encoding: Encoding,
) -> anyhow::Result<Option<Value>> {
    match encoding {
        Encoding::Json => {
            let mut line = Vec::with_capacity(256);
            let n = tokio::io::AsyncBufReadExt::read_until(reader, b'\n', &mut line).await?;
            if n == 0 {
                return Ok(None);
            }
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if line.is_empty() {
                return Ok(None);
            }
            let v: serde_json::Value = serde_json::from_slice(&line)?;
            Ok(Some(json_to_value(v)))
        }
        Encoding::BserV1 | Encoding::BserV2 => {
            // Read the magic bytes (re-verifying each PDU keeps the
            // implementation symmetric across sessions; v2 always emits
            // the magic per-PDU anyway).
            let first = match read_u8(reader).await? {
                Some(b) => b,
                None => return Ok(None),
            };
            if first != 0x00 {
                anyhow::bail!("expected BSER magic, got {first:#x}");
            }
            let version = read_u8(reader)
                .await?
                .ok_or_else(|| anyhow::anyhow!("EOF after BSER magic"))?;
            let actual = match version {
                0x01 => Encoding::BserV1,
                0x02 => {
                    let mut caps = [0u8; 4];
                    tokio::io::AsyncReadExt::read_exact(reader, &mut caps).await?;
                    Encoding::BserV2
                }
                other => anyhow::bail!("unknown BSER version {other:#x}"),
            };
            if actual != encoding {
                anyhow::bail!("client switched BSER version mid-session");
            }
            let length = read_bser_int(reader).await?;
            let mut payload = vec![0u8; length as usize];
            tokio::io::AsyncReadExt::read_exact(reader, &mut payload).await?;
            let (value, _) = bser_decode_value(&payload)?;
            Ok(Some(value))
        }
    }
}

async fn read_u8<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> anyhow::Result<Option<u8>> {
    let mut buf = [0u8; 1];
    match r.read_exact(&mut buf).await {
        Ok(_) => Ok(Some(buf[0])),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
        Err(e) => Err(e.into()),
    }
}

async fn read_bser_int<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> anyhow::Result<i64> {
    let tag = read_u8(r)
        .await?
        .ok_or_else(|| anyhow::anyhow!("EOF reading BSER int tag"))?;
    let len = match tag {
        bser::TAG_INT8 => 1,
        bser::TAG_INT16 => 2,
        bser::TAG_INT32 => 4,
        bser::TAG_INT64 => 8,
        other => anyhow::bail!("expected BSER int tag, got {other:#x}"),
    };
    let mut raw = vec![0u8; len];
    r.read_exact(&mut raw).await?;
    Ok(match tag {
        bser::TAG_INT8 => raw[0] as i8 as i64,
        bser::TAG_INT16 => i16::from_le_bytes([raw[0], raw[1]]) as i64,
        bser::TAG_INT32 => i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as i64,
        _ => i64::from_le_bytes([
            raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
        ]),
    })
}

fn bser_decode_value(payload: &[u8]) -> anyhow::Result<(Value, usize)> {
    // Borrow watchwoman-protocol's decode_pdu by faking a v1 header,
    // so we reuse a single decoder path.
    let mut framed = Vec::with_capacity(payload.len() + 5);
    framed.extend_from_slice(&bser::MAGIC_V1);
    // length tag — pick smallest fitting int
    let n = payload.len() as i64;
    if (i8::MIN as i64..=i8::MAX as i64).contains(&n) {
        framed.push(bser::TAG_INT8);
        framed.push(n as i8 as u8);
    } else if (i16::MIN as i64..=i16::MAX as i64).contains(&n) {
        framed.push(bser::TAG_INT16);
        framed.extend_from_slice(&(n as i16).to_le_bytes());
    } else if (i32::MIN as i64..=i32::MAX as i64).contains(&n) {
        framed.push(bser::TAG_INT32);
        framed.extend_from_slice(&(n as i32).to_le_bytes());
    } else {
        framed.push(bser::TAG_INT64);
        framed.extend_from_slice(&n.to_le_bytes());
    }
    framed.extend_from_slice(payload);
    let (v, consumed) = bser::decode_pdu(&framed)?;
    Ok((v, consumed))
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
