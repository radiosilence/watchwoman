use std::path::{Path, PathBuf};
use std::sync::Arc;

use watchwoman_protocol::Value;

use super::{obj, CommandError, CommandResult};
use crate::daemon::alloc;
use crate::daemon::state::DaemonState;

pub fn watch(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let path = arg_path(args)?;
    let canonical = std::fs::canonicalize(&path)
        .map_err(|e| CommandError::Internal(anyhow::anyhow!("canonicalize: {e}")))?;

    state
        .register_root(canonical.clone())
        .map_err(CommandError::Internal)?;
    Ok(obj([
        ("watch", Value::String(canonical.to_string_lossy().into())),
        ("watcher", Value::String(default_watcher_name().into())),
    ]))
}

pub fn watch_project(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let path = arg_path(args)?;
    let canonical = std::fs::canonicalize(&path)
        .map_err(|e| CommandError::Internal(anyhow::anyhow!("canonicalize: {e}")))?;

    let (root, relative_path) = resolve_project_root(&canonical);
    state
        .register_root(root.clone())
        .map_err(CommandError::Internal)?;

    let mut entries: Vec<(&str, Value)> = vec![
        ("watch", Value::String(root.to_string_lossy().into())),
        ("watcher", Value::String(default_watcher_name().into())),
    ];
    if let Some(rel) = relative_path {
        entries.push(("relative_path", Value::String(rel.to_string_lossy().into())));
    }
    let mut m = indexmap::IndexMap::with_capacity(entries.len());
    for (k, v) in entries {
        m.insert(k.to_owned(), v);
    }
    Ok(Value::Object(m))
}

pub fn watch_list(state: &Arc<DaemonState>) -> CommandResult {
    let roots: Vec<Value> = state
        .list_roots()
        .into_iter()
        .map(|p| Value::String(p.to_string_lossy().into()))
        .collect();
    Ok(obj([("roots", Value::Array(roots))]))
}

pub fn watch_del(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let path = arg_path(args)?;
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    let removed = state.unregister_root(&canonical);
    if removed {
        // Hand the freed file-tree pages back to the OS so RSS
        // reflects reality — see daemon::alloc for the long version.
        alloc::purge();
    }
    Ok(obj([
        ("watch-del", Value::Bool(removed)),
        ("root", Value::String(canonical.to_string_lossy().into())),
    ]))
}

pub fn watch_del_all(state: &Arc<DaemonState>) -> CommandResult {
    let drained = state.drain_roots();
    if !drained.is_empty() {
        alloc::purge();
    }
    let removed: Vec<Value> = drained
        .into_iter()
        .map(|p| Value::String(p.to_string_lossy().into()))
        .collect();
    Ok(obj([("roots", Value::Array(removed))]))
}

fn arg_path(args: &[Value]) -> Result<PathBuf, CommandError> {
    let s = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("expected a path argument".into()))?;
    Ok(PathBuf::from(s))
}

fn default_watcher_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "fsevents"
    } else if cfg!(target_os = "linux") {
        "inotify"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "kqueue"
    }
}

const ROOT_MARKERS: &[&str] = &[
    ".watchmanconfig",
    ".git",
    ".hg",
    ".svn",
    ".jj",
    "package.json",
    "Cargo.toml",
    "mix.exs",
    "pyproject.toml",
    "go.mod",
];

fn resolve_project_root(start: &Path) -> (PathBuf, Option<PathBuf>) {
    let mut cur = start;
    while let Some(parent) = cur.parent() {
        if has_root_marker(cur) {
            let rel = start.strip_prefix(cur).ok().and_then(|r| {
                if r.as_os_str().is_empty() {
                    None
                } else {
                    Some(r.to_path_buf())
                }
            });
            return (cur.to_path_buf(), rel);
        }
        cur = parent;
    }
    (start.to_path_buf(), None)
}

fn has_root_marker(dir: &Path) -> bool {
    ROOT_MARKERS.iter().any(|m| dir.join(m).exists())
}
