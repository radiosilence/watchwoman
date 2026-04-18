//! Per-connection session state.
//!
//! The session owns the write side of the socket (via an mpsc queue)
//! and the encoding negotiated on first byte.  Commands that need to
//! push unilateral PDUs — subscribe, state-enter broadcasts — clone
//! the session handle and send into it.
//!
//! When the connection drops, the writer task exits, the mpsc tx
//! copies all fail, and any subscription push tasks notice and tear
//! themselves down.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use watchwoman_protocol::{Encoding, Value};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct Session(Arc<SessionInner>);

struct SessionInner {
    id: u64,
    encoding: Encoding,
    tx: mpsc::UnboundedSender<Value>,
}

impl Session {
    pub fn new(encoding: Encoding, tx: mpsc::UnboundedSender<Value>) -> Self {
        let id = NEXT_SESSION_ID.fetch_add(1, Ordering::AcqRel);
        Self(Arc::new(SessionInner { id, encoding, tx }))
    }

    pub fn id(&self) -> u64 {
        self.0.id
    }

    pub fn encoding(&self) -> Encoding {
        self.0.encoding
    }

    /// Enqueue a PDU to be written on the connection.  Returns
    /// `Err` when the peer has disconnected.
    pub fn send(&self, value: Value) -> Result<(), SessionClosed> {
        self.0.tx.send(value).map_err(|_| SessionClosed)
    }

    pub fn is_closed(&self) -> bool {
        self.0.tx.is_closed()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("session closed")]
pub struct SessionClosed;
