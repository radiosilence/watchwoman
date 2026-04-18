//! Integration-test harness shared between every `tests/*.rs` file.
//!
//! The harness spawns whichever binary is under test (watchwoman by
//! default, real watchman if `WATCHWOMAN_UNDER_TEST=watchman`), gives it
//! an isolated socket + state dir, and hands back a [`Client`] that
//! speaks the JSON PDU protocol.
//!
//! Every test should be authored so it passes on **both** binaries —
//! that's the parity guarantee.

pub mod client;
pub mod harness;
pub mod scratch;

pub use client::Client;
pub use harness::{Harness, TargetBinary};
pub use scratch::Scratch;
