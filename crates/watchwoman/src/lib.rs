//! watchwoman — a drop-in watchman replacement.
//!
//! The crate is both a library (for integration tests that need to poke
//! at internals) and a binary (the installed daemon/CLI).

pub mod cli;
pub mod commands;
pub mod daemon;
pub mod query;
pub mod sock;
pub mod watcher;

pub use watchwoman_protocol as protocol;

/// Version string reported by the `version` command. Uses watchman's
/// YYYY.MM.DD.BB format so capability probes keep working.
pub const WATCHWOMAN_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Watchman version we advertise over the wire.  Upstream clients gate
/// features on this string, so we keep the shape identical.
pub const WATCHMAN_COMPAT_VERSION: &str = "2026.03.30.00";
