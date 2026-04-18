//! BSER — watchman's binary serialisation.
//!
//! Two framing formats share one payload grammar.
//!
//! Framing (v1):
//!
//! ```text
//! [00 01]            magic
//! [int-tagged len]   total payload size in bytes, BSER-encoded integer
//! [payload]          BSER-encoded Value
//! ```
//!
//! Framing (v2) inserts a 4-byte little-endian capability bitmask
//! between the magic and the length. Capability bits:
//!
//! - `0x1` disable-unicode — emit every string as bytestring tag 0x02.
//! - `0x2` disable-unicode-for-errors — only error payloads as bytestring.
//!
//! Payload grammar — tag bytes followed by variable-length data:
//!
//! ```text
//! 0x00 array    [tag][int count][value...]
//! 0x01 object   [tag][int count][string key, value]...
//! 0x02 string   [tag][int len][bytes]     (bytestring; may or may not be utf-8)
//! 0x03 int8     [tag][i8]
//! 0x04 int16    [tag][i16 LE]
//! 0x05 int32    [tag][i32 LE]
//! 0x06 int64    [tag][i64 LE]
//! 0x07 real     [tag][f64 LE]
//! 0x08 true
//! 0x09 false
//! 0x0a null
//! 0x0b template [tag][keys-as-array][int row_count][row values]
//! 0x0c skip     (only valid inside a template row)
//! 0x0d utf8     [tag][int len][bytes]     (v2; semantically identical to 0x02)
//! ```

use std::io::{Read, Write};

use indexmap::IndexMap;

use crate::{Encoding, ProtocolError, Result, Value};

pub const MAGIC_V1: [u8; 2] = [0x00, 0x01];
pub const MAGIC_V2: [u8; 2] = [0x00, 0x02];

pub const TAG_ARRAY: u8 = 0x00;
pub const TAG_OBJECT: u8 = 0x01;
pub const TAG_STRING: u8 = 0x02;
pub const TAG_INT8: u8 = 0x03;
pub const TAG_INT16: u8 = 0x04;
pub const TAG_INT32: u8 = 0x05;
pub const TAG_INT64: u8 = 0x06;
pub const TAG_REAL: u8 = 0x07;
pub const TAG_TRUE: u8 = 0x08;
pub const TAG_FALSE: u8 = 0x09;
pub const TAG_NULL: u8 = 0x0A;
pub const TAG_TEMPLATE: u8 = 0x0B;
pub const TAG_SKIP: u8 = 0x0C;
pub const TAG_UTF8: u8 = 0x0D;

/// Encode a complete BSER PDU (magic + length + payload).
pub fn encode_pdu(value: &Value, version: Encoding) -> Result<Vec<u8>> {
    let mut payload = Vec::with_capacity(256);
    write_value(&mut payload, value)?;

    let mut out = Vec::with_capacity(2 + 9 + payload.len());
    match version {
        Encoding::BserV1 => out.extend_from_slice(&MAGIC_V1),
        Encoding::BserV2 => {
            out.extend_from_slice(&MAGIC_V2);
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        Encoding::Json => {
            return Err(ProtocolError::Bser(
                "encode_pdu called with Encoding::Json".into(),
            ))
        }
    }
    write_int(&mut out, payload.len() as i64);
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Decode one PDU from a byte slice.  Returns the decoded value and the
/// number of bytes consumed (useful for stream decoders that keep a
/// rolling buffer).
pub fn decode_pdu(buf: &[u8]) -> Result<(Value, usize)> {
    if buf.len() < 2 {
        return Err(ProtocolError::Truncated);
    }
    let header_len = match &buf[..2] {
        b if *b == MAGIC_V1 => 2,
        b if *b == MAGIC_V2 => {
            if buf.len() < 6 {
                return Err(ProtocolError::Truncated);
            }
            6
        }
        other => return Err(ProtocolError::UnknownEncoding(other[0])),
    };
    let (payload_len, len_size) = read_int(&buf[header_len..])?;
    let payload_start = header_len + len_size;
    let payload_end = payload_start + payload_len as usize;
    if buf.len() < payload_end {
        return Err(ProtocolError::Truncated);
    }
    let (value, consumed) = read_value(&buf[payload_start..payload_end])?;
    if consumed != payload_len as usize {
        return Err(ProtocolError::Bser(format!(
            "payload length mismatch: declared {payload_len}, consumed {consumed}"
        )));
    }
    Ok((value, payload_end))
}

/// Read one PDU from a synchronous reader. Returns `Ok(None)` on clean
/// EOF (no bytes available), `Err(Truncated)` on a partial PDU.
pub fn read_pdu<R: Read>(r: &mut R) -> Result<Option<(Value, Encoding)>> {
    let mut magic = [0u8; 2];
    match r.read_exact(&mut magic) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let version = match magic {
        MAGIC_V1 => Encoding::BserV1,
        MAGIC_V2 => {
            let mut caps = [0u8; 4];
            r.read_exact(&mut caps)?;
            Encoding::BserV2
        }
        other => return Err(ProtocolError::UnknownEncoding(other[0])),
    };
    let length = read_int_from(r)? as usize;
    let mut payload = vec![0u8; length];
    r.read_exact(&mut payload)?;
    let (value, _) = read_value(&payload)?;
    Ok(Some((value, version)))
}

/// Write a PDU to a synchronous writer.
pub fn write_pdu<W: Write>(w: &mut W, value: &Value, version: Encoding) -> Result<()> {
    let bytes = encode_pdu(value, version)?;
    w.write_all(&bytes)?;
    Ok(())
}

// ------- payload writers -------

fn write_int(out: &mut Vec<u8>, n: i64) {
    if (i8::MIN as i64..=i8::MAX as i64).contains(&n) {
        out.push(TAG_INT8);
        out.push(n as i8 as u8);
    } else if (i16::MIN as i64..=i16::MAX as i64).contains(&n) {
        out.push(TAG_INT16);
        out.extend_from_slice(&(n as i16).to_le_bytes());
    } else if (i32::MIN as i64..=i32::MAX as i64).contains(&n) {
        out.push(TAG_INT32);
        out.extend_from_slice(&(n as i32).to_le_bytes());
    } else {
        out.push(TAG_INT64);
        out.extend_from_slice(&n.to_le_bytes());
    }
}

fn write_value(out: &mut Vec<u8>, v: &Value) -> Result<()> {
    match v {
        Value::Null => out.push(TAG_NULL),
        Value::Bool(true) => out.push(TAG_TRUE),
        Value::Bool(false) => out.push(TAG_FALSE),
        Value::Int(i) => write_int(out, *i),
        Value::Real(f) => {
            out.push(TAG_REAL);
            out.extend_from_slice(&f.to_le_bytes());
        }
        Value::String(s) => {
            out.push(TAG_STRING);
            write_int(out, s.len() as i64);
            out.extend_from_slice(s.as_bytes());
        }
        Value::Bytes(b) => {
            out.push(TAG_STRING);
            write_int(out, b.len() as i64);
            out.extend_from_slice(b);
        }
        Value::Array(a) => {
            out.push(TAG_ARRAY);
            write_int(out, a.len() as i64);
            for item in a {
                write_value(out, item)?;
            }
        }
        Value::Object(o) => {
            out.push(TAG_OBJECT);
            write_int(out, o.len() as i64);
            for (k, val) in o {
                out.push(TAG_STRING);
                write_int(out, k.len() as i64);
                out.extend_from_slice(k.as_bytes());
                write_value(out, val)?;
            }
        }
        Value::Template { keys, rows } => {
            out.push(TAG_TEMPLATE);
            // Keys as an array of strings, then row count, then rows in
            // flat order (keys.len() values per row, with SKIP tags
            // reserved for absent fields — we serialise Null as SKIP so
            // absent fields round-trip cleanly.)
            out.push(TAG_ARRAY);
            write_int(out, keys.len() as i64);
            for k in keys {
                out.push(TAG_STRING);
                write_int(out, k.len() as i64);
                out.extend_from_slice(k.as_bytes());
            }
            write_int(out, rows.len() as i64);
            for row in rows {
                for val in row {
                    if matches!(val, Value::Null) {
                        out.push(TAG_SKIP);
                    } else {
                        write_value(out, val)?;
                    }
                }
            }
        }
    }
    Ok(())
}

// ------- payload readers -------

fn read_int(buf: &[u8]) -> Result<(i64, usize)> {
    if buf.is_empty() {
        return Err(ProtocolError::Truncated);
    }
    match buf[0] {
        TAG_INT8 => {
            need(buf, 2)?;
            Ok((buf[1] as i8 as i64, 2))
        }
        TAG_INT16 => {
            need(buf, 3)?;
            let v = i16::from_le_bytes([buf[1], buf[2]]);
            Ok((v as i64, 3))
        }
        TAG_INT32 => {
            need(buf, 5)?;
            let v = i32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]);
            Ok((v as i64, 5))
        }
        TAG_INT64 => {
            need(buf, 9)?;
            let v = i64::from_le_bytes([
                buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8],
            ]);
            Ok((v, 9))
        }
        other => Err(ProtocolError::Bser(format!(
            "expected int tag, got {other:#x}"
        ))),
    }
}

fn read_int_from<R: Read>(r: &mut R) -> Result<i64> {
    let mut tag = [0u8; 1];
    r.read_exact(&mut tag)?;
    let n_bytes: usize = match tag[0] {
        TAG_INT8 => 1,
        TAG_INT16 => 2,
        TAG_INT32 => 4,
        TAG_INT64 => 8,
        other => {
            return Err(ProtocolError::Bser(format!(
                "expected int tag, got {other:#x}"
            )))
        }
    };
    let mut raw = vec![0u8; n_bytes];
    r.read_exact(&mut raw)?;
    Ok(match tag[0] {
        TAG_INT8 => raw[0] as i8 as i64,
        TAG_INT16 => i16::from_le_bytes([raw[0], raw[1]]) as i64,
        TAG_INT32 => i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as i64,
        _ => i64::from_le_bytes([
            raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
        ]),
    })
}

fn read_value(buf: &[u8]) -> Result<(Value, usize)> {
    if buf.is_empty() {
        return Err(ProtocolError::Truncated);
    }
    match buf[0] {
        TAG_NULL => Ok((Value::Null, 1)),
        TAG_TRUE => Ok((Value::Bool(true), 1)),
        TAG_FALSE => Ok((Value::Bool(false), 1)),
        TAG_INT8 | TAG_INT16 | TAG_INT32 | TAG_INT64 => {
            let (n, consumed) = read_int(buf)?;
            Ok((Value::Int(n), consumed))
        }
        TAG_REAL => {
            need(buf, 9)?;
            let bytes: [u8; 8] = buf[1..9].try_into().expect("slice length checked");
            Ok((Value::Real(f64::from_le_bytes(bytes)), 9))
        }
        TAG_STRING | TAG_UTF8 => {
            let (len, lenlen) = read_int(&buf[1..])?;
            let start = 1 + lenlen;
            let end = start + len as usize;
            need(buf, end)?;
            let slice = &buf[start..end];
            let value = match std::str::from_utf8(slice) {
                Ok(s) => Value::String(s.to_owned()),
                Err(_) => Value::Bytes(slice.to_vec()),
            };
            Ok((value, end))
        }
        TAG_ARRAY => {
            let (count, clen) = read_int(&buf[1..])?;
            let mut pos = 1 + clen;
            let mut items = Vec::with_capacity(count.max(0) as usize);
            for _ in 0..count {
                let (v, consumed) = read_value(&buf[pos..])?;
                items.push(v);
                pos += consumed;
            }
            Ok((Value::Array(items), pos))
        }
        TAG_OBJECT => {
            let (count, clen) = read_int(&buf[1..])?;
            let mut pos = 1 + clen;
            let mut obj = IndexMap::with_capacity(count.max(0) as usize);
            for _ in 0..count {
                let (k, kconsumed) = read_value(&buf[pos..])?;
                pos += kconsumed;
                let key = match k {
                    Value::String(s) => s,
                    Value::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
                    other => {
                        return Err(ProtocolError::Bser(format!(
                            "object key must be string, got {other:?}"
                        )))
                    }
                };
                let (v, vconsumed) = read_value(&buf[pos..])?;
                pos += vconsumed;
                obj.insert(key, v);
            }
            Ok((Value::Object(obj), pos))
        }
        TAG_TEMPLATE => {
            let (keys_val, keys_consumed) = read_value(&buf[1..])?;
            let keys: Vec<String> = match keys_val {
                Value::Array(a) => a
                    .into_iter()
                    .map(|v| match v {
                        Value::String(s) => Ok(s),
                        Value::Bytes(b) => Ok(String::from_utf8_lossy(&b).into_owned()),
                        other => Err(ProtocolError::Bser(format!(
                            "template key must be string, got {other:?}"
                        ))),
                    })
                    .collect::<Result<Vec<_>>>()?,
                _ => return Err(ProtocolError::Bser("template keys must be an array".into())),
            };
            let mut pos = 1 + keys_consumed;
            let (row_count, rclen) = read_int(&buf[pos..])?;
            pos += rclen;
            let mut rows = Vec::with_capacity(row_count.max(0) as usize);
            for _ in 0..row_count {
                let mut row = Vec::with_capacity(keys.len());
                for _ in 0..keys.len() {
                    if buf.get(pos) == Some(&TAG_SKIP) {
                        row.push(Value::Null);
                        pos += 1;
                    } else {
                        let (v, vconsumed) = read_value(&buf[pos..])?;
                        pos += vconsumed;
                        row.push(v);
                    }
                }
                rows.push(row);
            }
            Ok((Value::Template { keys, rows }, pos))
        }
        TAG_SKIP => Err(ProtocolError::Bser(
            "unexpected SKIP tag outside a template".into(),
        )),
        other => Err(ProtocolError::Bser(format!("unknown BSER tag {other:#x}"))),
    }
}

fn need(buf: &[u8], size: usize) -> Result<()> {
    if buf.len() < size {
        Err(ProtocolError::Truncated)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: Value, version: Encoding) {
        let bytes = encode_pdu(&v, version).unwrap();
        let (decoded, consumed) = decode_pdu(&bytes).unwrap();
        assert_eq!(consumed, bytes.len(), "consumed all bytes");
        assert_eq!(decoded, v);
    }

    #[test]
    fn primitives_v1() {
        for v in [
            Value::Null,
            Value::Bool(true),
            Value::Bool(false),
            Value::Int(0),
            Value::Int(127),
            Value::Int(-128),
            Value::Int(32_000),
            Value::Int(-32_000),
            Value::Int(2_000_000_000),
            Value::Int(i64::MAX),
            Value::Int(i64::MIN),
            Value::Real(0.0),
            Value::Real(-1.5),
            Value::Real(f64::INFINITY),
            Value::String("".into()),
            Value::String("hello".into()),
            Value::String("sürprise".into()),
        ] {
            roundtrip(v, Encoding::BserV1);
        }
    }

    #[test]
    fn nested_v2() {
        let mut obj = IndexMap::new();
        obj.insert("watch".into(), Value::String("/tmp".into()));
        obj.insert(
            "files".into(),
            Value::Array(vec![Value::String("a.rs".into()), Value::Int(42)]),
        );
        obj.insert("ok".into(), Value::Bool(true));
        roundtrip(Value::Object(obj), Encoding::BserV2);
    }

    #[test]
    fn template_with_skip() {
        let keys = vec!["name".to_owned(), "size".to_owned()];
        let rows = vec![
            vec![Value::String("a".into()), Value::Int(1)],
            vec![Value::String("b".into()), Value::Null], // SKIP
        ];
        roundtrip(Value::Template { keys, rows }, Encoding::BserV1);
    }

    #[test]
    fn streaming_read_write() {
        let v = Value::Array(vec![
            Value::String("version".into()),
            Value::Object(IndexMap::from_iter([(
                "required".to_owned(),
                Value::Array(vec![Value::String("cmd-query".into())]),
            )])),
        ]);
        let mut buf = Vec::new();
        write_pdu(&mut buf, &v, Encoding::BserV2).unwrap();
        let mut cur = std::io::Cursor::new(buf);
        let (decoded, enc) = read_pdu(&mut cur).unwrap().unwrap();
        assert_eq!(decoded, v);
        assert_eq!(enc, Encoding::BserV2);
    }

    #[test]
    fn eof_returns_none() {
        let mut cur = std::io::Cursor::new(Vec::<u8>::new());
        assert!(read_pdu(&mut cur).unwrap().is_none());
    }

    #[test]
    fn bad_magic_errors() {
        let mut cur = std::io::Cursor::new(vec![0xff, 0xff, 0, 0]);
        let err = read_pdu(&mut cur).unwrap_err();
        assert!(matches!(err, ProtocolError::UnknownEncoding(_)));
    }

    #[test]
    fn encode_v1_framing_shape() {
        let bytes = encode_pdu(&Value::Null, Encoding::BserV1).unwrap();
        assert_eq!(&bytes[..2], &MAGIC_V1);
        // length tag (int8=0x03) then 0x01 (one byte of payload: TAG_NULL)
        assert_eq!(bytes[2], TAG_INT8);
        assert_eq!(bytes[3], 1);
        assert_eq!(bytes[4], TAG_NULL);
    }

    #[test]
    fn encode_v2_has_capability_bytes() {
        let bytes = encode_pdu(&Value::Null, Encoding::BserV2).unwrap();
        assert_eq!(&bytes[..2], &MAGIC_V2);
        assert_eq!(&bytes[2..6], &[0, 0, 0, 0]);
    }
}
