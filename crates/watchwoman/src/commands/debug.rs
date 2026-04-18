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
    let entries = crate::daemon::watcher::initial_scan(&root.path, &root.ignore_dirs);
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

pub fn get_asserted_states(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root = resolve_root(state, args)?;
    let states: Vec<Value> = root
        .asserted_states
        .read()
        .iter()
        .map(|s| Value::String(s.clone()))
        .collect();
    Ok(obj([
        ("root", Value::String(root.path.to_string_lossy().into())),
        ("states", Value::Array(states)),
    ]))
}

pub fn get_subscriptions(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root = resolve_root(state, args)?;
    let subs: Vec<Value> = root
        .subscriptions()
        .into_iter()
        .map(|s| {
            let mut m = IndexMap::new();
            m.insert("name".into(), Value::String(s.name));
            m.insert("query".into(), s.query);
            Value::Object(m)
        })
        .collect();
    Ok(obj([("subscriptions", Value::Array(subs))]))
}

pub fn root_status(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root = resolve_root(state, args)?;
    let tree_len = root.tree.read().len() as i64;
    Ok(obj([
        ("root", Value::String(root.path.to_string_lossy().into())),
        ("num_files", Value::Int(tree_len)),
        ("clock", Value::String(root.clock_string())),
    ]))
}

pub fn status(state: &Arc<DaemonState>) -> CommandResult {
    let roots: Vec<Value> = state
        .list_roots()
        .into_iter()
        .map(|p| Value::String(p.to_string_lossy().into()))
        .collect();
    Ok(obj([
        ("roots", Value::Array(roots)),
        ("pid", Value::Int(std::process::id() as i64)),
    ]))
}

pub fn watcher_info(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root = resolve_root(state, args)?;
    let name = if cfg!(target_os = "macos") {
        "fsevents"
    } else if cfg!(target_os = "linux") {
        "inotify"
    } else {
        "kqueue"
    };
    Ok(obj([
        ("root", Value::String(root.path.to_string_lossy().into())),
        ("watcher", Value::String(name.into())),
    ]))
}

pub fn contenthash(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root = resolve_root(state, args)?;
    Ok(obj([
        ("root", Value::String(root.path.to_string_lossy().into())),
        ("enabled", Value::Bool(true)),
    ]))
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
