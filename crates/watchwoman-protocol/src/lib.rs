//! Watchman wire protocol codecs.
//!
//! Two encodings live side by side: newline-delimited JSON (for CLI users
//! and scripts) and BSER, the binary encoding watchman ships for its
//! language bindings. Both produce and consume the same [`Value`] tree so
//! the daemon stays codec-agnostic.

pub mod bser;
pub mod json;
pub mod value;

pub use value::{Value, ValueRef};

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("bser: {0}")]
    Bser(String),
    #[error("truncated PDU")]
    Truncated,
    #[error("unknown encoding tag: {0:#x}")]
    UnknownEncoding(u8),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

/// Wire encoding selected during the handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Json,
    BserV1,
    BserV2,
}
