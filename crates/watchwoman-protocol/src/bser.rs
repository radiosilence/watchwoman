//! BSER — watchman's binary serialization.
//!
//! Reverse-engineered from `watchman/cppclient` and `watchman/bser.cpp`
//! in the upstream source. See [`reference`](../reference/watchman/) for
//! the authoritative implementation.
//!
//! BSER framing:
//!   [0x00 0x01]                       magic (v1) — or [0x00 0x02] for v2
//!   <encoded-int length-of-pdu>       BSER-encoded integer giving the
//!                                     byte length of the payload that
//!                                     follows this header
//!   <payload>                         BSER-encoded Value
//!
//! v2 additionally prepends a 4-byte capability bitmask between the magic
//! and the length. The type tags below are shared across versions.
//!
//! Type tags:
//!   0x00 array
//!   0x01 object
//!   0x02 string (utf-8)
//!   0x03 int8
//!   0x04 int16
//!   0x05 int32
//!   0x06 int64
//!   0x07 real (f64, little endian)
//!   0x08 true
//!   0x09 false
//!   0x0A null
//!   0x0B template
//!   0x0C skip (used inside templates for absent fields)
//!   0x0D utf8-string (v2 only; otherwise equivalent to 0x02)
//!
//! The full decoder/encoder pair is not implemented yet — see the
//! parity-tracking issue. The skeleton below defines the public API so
//! the daemon can already depend on it.

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

/// Encode a value as a complete BSER PDU (magic + length + payload).
pub fn encode_pdu(_value: &Value, _version: Encoding) -> Result<Vec<u8>> {
    Err(ProtocolError::Bser("encode not yet implemented".into()))
}

/// Decode one PDU from `buf`. Returns the value and the number of bytes
/// consumed.
pub fn decode_pdu(_buf: &[u8]) -> Result<(Value, usize)> {
    Err(ProtocolError::Bser("decode not yet implemented".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_is_stubbed() {
        assert!(encode_pdu(&Value::Null, Encoding::BserV1).is_err());
    }
}
