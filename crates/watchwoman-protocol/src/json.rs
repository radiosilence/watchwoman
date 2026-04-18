use std::io::{BufRead, Write};

use indexmap::IndexMap;

use crate::{ProtocolError, Result, Value};

/// Encode a value as a watchman JSON PDU (one line, newline-terminated).
pub fn encode_pdu<W: Write>(w: &mut W, value: &Value) -> Result<()> {
    write_value(w, value)?;
    w.write_all(b"\n")?;
    Ok(())
}

/// Read a single newline-terminated JSON PDU from `r`.
/// Returns `Ok(None)` when the reader is cleanly at EOF.
pub fn read_pdu<R: BufRead>(r: &mut R) -> Result<Option<Value>> {
    let mut line = Vec::with_capacity(256);
    let n = r.read_until(b'\n', &mut line)?;
    if n == 0 {
        return Ok(None);
    }
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    if line.is_empty() {
        return Err(ProtocolError::Truncated);
    }
    let v: serde_json::Value = serde_json::from_slice(&line)?;
    Ok(Some(from_serde(v)))
}

fn write_value<W: Write>(w: &mut W, value: &Value) -> Result<()> {
    let v = to_serde(value);
    serde_json::to_writer(w, &v)?;
    Ok(())
}

fn to_serde(value: &Value) -> serde_json::Value {
    use serde_json::{Number, Value as J};
    match value {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => J::Number(Number::from(*i)),
        Value::Real(f) => Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
        Value::String(s) => J::String(s.clone()),
        Value::Bytes(b) => J::String(String::from_utf8_lossy(b).into_owned()),
        Value::Array(a) => J::Array(a.iter().map(to_serde).collect()),
        Value::Object(o) => {
            let mut map = serde_json::Map::new();
            for (k, v) in o {
                map.insert(k.clone(), to_serde(v));
            }
            J::Object(map)
        }
        Value::Template { keys, rows } => {
            // JSON has no template concept; expand into an array of objects.
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let mut obj = serde_json::Map::new();
                for (k, v) in keys.iter().zip(row.iter()) {
                    obj.insert(k.clone(), to_serde(v));
                }
                out.push(J::Object(obj));
            }
            J::Array(out)
        }
    }
}

fn from_serde(value: serde_json::Value) -> Value {
    use serde_json::Value as J;
    match value {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                Value::Int(u as i64)
            } else if let Some(f) = n.as_f64() {
                Value::Real(f)
            } else {
                Value::Null
            }
        }
        J::String(s) => Value::String(s),
        J::Array(a) => Value::Array(a.into_iter().map(from_serde).collect()),
        J::Object(o) => {
            let mut map = IndexMap::with_capacity(o.len());
            for (k, v) in o {
                map.insert(k, from_serde(v));
            }
            Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_basic_pdu() {
        let pdu = Value::Array(vec![Value::String("version".into())]);
        let mut buf = Vec::new();
        encode_pdu(&mut buf, &pdu).unwrap();
        assert_eq!(buf.last(), Some(&b'\n'));
        let mut cur = std::io::Cursor::new(buf);
        let decoded = read_pdu(&mut cur).unwrap().unwrap();
        assert_eq!(decoded, pdu);
    }

    #[test]
    fn eof_returns_none() {
        let mut cur = std::io::Cursor::new(Vec::<u8>::new());
        assert!(read_pdu(&mut cur).unwrap().is_none());
    }
}
