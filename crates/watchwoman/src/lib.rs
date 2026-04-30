//! watchwoman — a drop-in watchman replacement.

// jemalloc on every platform that jemalloc supports.  See the
// `target.'cfg(...)'.dependencies` block in Cargo.toml for rationale;
// short version: macOS' system allocator and glibc both hold freed
// pages indefinitely, so RSS doesn't drop after a `watch-del` even
// though the file-tree memory is logically gone.  jemalloc, plus the
// purge we call from `daemon::alloc::purge`, fixes that.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

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
