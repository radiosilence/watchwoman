//! `debug-*` commands.  Watchman exposes these as escape hatches for
//! CI systems that want to poke the daemon's state; we keep them
//! available for drop-in compat but implement the safe ones only.

use std::path::PathBuf;
use std::sync::Arc;

use indexmap::IndexMap;
use watchwoman_protocol::Value;

use super::{obj, CommandError, CommandResult};
use crate::daemon::root::Root;
use crate::daemon::state::DaemonState;

pub fn recrawl(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root = resolve_root(state, args)?;
    // Re-seed the tree from a fresh scan of the disk. Useful when
    // something outside the kernel's event stream (e.g. a bind-mount
    // or a network fs that coalesced) has drifted from reality.
    let entries = crate::daemon::watcher::initial_scan(&root.path);
    let count = entries.len();
    root.seed(entries);
    Ok(obj([
        ("recrawled", Value::Bool(true)),
        ("root", Value::String(root.path.to_string_lossy().into())),
        ("files", Value::Int(count as i64)),
    ]))
}

pub fn ageout(_state: &Arc<DaemonState>, _args: &[Value]) -> CommandResult {
    // Watchman's ageout sweeper retires absent entries after N seconds.
    // Watchwoman's tree doesn't grow unboundedly — the watcher marks
    // entries `exists: false` but doesn't keep history, so there's
    // nothing to age out.  Return the shape clients expect.
    Ok(obj([
        ("ageout", Value::Bool(true)),
        ("files", Value::Int(0)),
    ]))
}

pub fn show_cursors(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root = resolve_root(state, args)?;
    let mut cursors = IndexMap::new();
    for (name, tick) in root.cursors() {
        cursors.insert(format!("n:{name}"), Value::String(root.clock.encode(tick)));
    }
    Ok(obj([("cursors", Value::Object(cursors))]))
}

fn resolve_root(state: &Arc<DaemonState>, args: &[Value]) -> Result<Arc<Root>, CommandError> {
    let root_str = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("debug command requires a root".into()))?;
    let root_path = std::fs::canonicalize(root_str).unwrap_or_else(|_| PathBuf::from(root_str));
    state
        .root(&root_path)
        .ok_or_else(|| CommandError::UnknownRoot(root_path.to_string_lossy().into()))
}
