//! watchwoman — a drop-in watchman replacement.

pub mod cli;
pub mod commands;
pub mod daemon;
pub mod query;
pub mod sock;

pub use watchwoman_protocol as protocol;

pub const WATCHWOMAN_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Version reported to clients. Upstream tools gate features on this
/// string, so we quote a real watchman release date.
pub const WATCHMAN_COMPAT_VERSION: &str = "2026.03.30.00";
