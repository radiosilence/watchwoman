//! notify-backed filesystem watcher.  One watcher per root; coalesces
//! events into tick batches before the tree sees them.
//!
//! We intentionally do not stream every event individually — that's
//! watchman's most common source of "too many inotify watches" and
//! "recrawl" noise.  Batching at 5 ms matches watchman's default
//! settle period well enough for CLI tools.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::root::{PathChange, Root, WatcherCommand};

const SETTLE: Duration = Duration::from_millis(5);
const MAX_BATCH: usize = 1024;

pub fn spawn(root: Arc<Root>, mut cmd_rx: mpsc::UnboundedReceiver<WatcherCommand>) {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<notify::Result<Event>>();

    let root_path = root.path.clone();
    let watcher_handle = tokio::task::spawn_blocking(move || {
        let mut watcher: RecommendedWatcher = match notify::recommended_watcher(move |res| {
            let _ = event_tx.send(res);
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(?e, "failed to create watcher");
                return;
            }
        };
        if let Err(e) = watcher.watch(&root_path, RecursiveMode::Recursive) {
            tracing::warn!(?e, "failed to watch root");
        }
        // Park the watcher thread — dropping this task's handle drops
        // the watcher, and we rely on cmd_rx shutdown to park.
        std::thread::park();
    });

    let root_for_events = root.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => match cmd {
                    Some(WatcherCommand::Shutdown) | None => break,
                },
                first = event_rx.recv() => {
                    let Some(first) = first else { break };
                    let mut batch = Vec::with_capacity(8);
                    collect_event(&root_for_events, first, &mut batch);
                    let deadline = tokio::time::Instant::now() + SETTLE;
                    while batch.len() < MAX_BATCH {
                        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                        if remaining.is_zero() { break; }
                        match tokio::time::timeout(remaining, event_rx.recv()).await {
                            Ok(Some(next)) => collect_event(&root_for_events, next, &mut batch),
                            Ok(None) | Err(_) => break,
                        }
                    }
                    if !batch.is_empty() {
                        root_for_events.apply_changes(batch);
                    }
                }
            }
        }
        watcher_handle.abort();
    });
}

fn collect_event(root: &Root, event: notify::Result<Event>, out: &mut Vec<PathChange>) {
    let event = match event {
        Ok(ev) => ev,
        Err(e) => {
            tracing::debug!(?e, "notify error");
            return;
        }
    };
    for abs in &event.paths {
        let Some(rel) = abs.strip_prefix(&root.path).ok().map(PathBuf::from) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        if should_ignore(&rel, &root.ignore_dirs) {
            continue;
        }
        match event.kind {
            EventKind::Remove(_) => {
                out.push(PathChange::Remove { rel });
            }
            _ => {
                let Ok(metadata) = std::fs::symlink_metadata(abs) else {
                    // File disappeared before we could stat — treat as removal.
                    out.push(PathChange::Remove { rel });
                    continue;
                };
                let symlink_target = if metadata.is_symlink() {
                    std::fs::read_link(abs)
                        .ok()
                        .map(|p| p.to_string_lossy().into_owned())
                } else {
                    None
                };
                out.push(PathChange::Upsert {
                    rel,
                    metadata,
                    symlink_target,
                });
            }
        }
    }
}

pub(crate) fn initial_scan(
    root: &Path,
    ignore: &[String],
) -> Vec<(PathBuf, std::fs::Metadata, Option<String>)> {
    // Manual recursive walk so we can prune at the directory level —
    // descending into `node_modules` before filtering was dominating
    // the initial scan on any large JS project.
    let mut out = Vec::new();
    walk(root, Path::new(""), ignore, &mut out, false);
    out
}

/// `shallow` is true once we've descended into a VCS directory
/// (`.git`/`.hg`/`.svn`).  Matches real watchman's behaviour: the
/// VCS dir itself and its immediate contents are reported, but we
/// don't recurse deeper into them.  `ignore_dirs` from
/// `.watchmanconfig` is applied at every depth and prunes the
/// subtree entirely.
fn walk(
    abs_dir: &std::path::Path,
    rel_dir: &std::path::Path,
    ignore: &[String],
    out: &mut Vec<(PathBuf, std::fs::Metadata, Option<String>)>,
    shallow: bool,
) {
    let Ok(read) = std::fs::read_dir(abs_dir) else {
        return;
    };
    for entry in read.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let rel = rel_dir.join(&name);
        if ignore.iter().any(|d| d == name_str.as_ref()) {
            continue;
        }
        let abs = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&abs) else {
            continue;
        };
        let symlink_target = if metadata.is_symlink() {
            std::fs::read_link(&abs)
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        };
        let is_real_dir = metadata.is_dir() && !metadata.file_type().is_symlink();
        let is_vcs = IGNORE_VCS.contains(&name_str.as_ref());
        out.push((rel.clone(), metadata, symlink_target));
        if is_real_dir && !shallow {
            walk(&abs, &rel, ignore, out, is_vcs);
        }
    }
}

/// VCS directory names whose immediate contents are reported but
/// whose subdirectories are not recursed into.  Matches upstream
/// watchman's behaviour for `.git` etc: you see `.git/HEAD` and
/// `.git/hooks` but not `.git/hooks/pre-commit.sample`.
const IGNORE_VCS: &[&str] = &[".git", ".hg", ".svn"];

/// Used by the fs-event code path: `extra` is `ignore_dirs` from
/// `.watchmanconfig`.  The initial scan does the VCS-shallow thing
/// inside [`walk`] directly; for ad-hoc events we fall back to
/// this coarse check.
fn should_ignore(rel: &std::path::Path, extra: &[String]) -> bool {
    let comps: Vec<_> = rel.components().collect();
    for (i, comp) in comps.iter().enumerate() {
        let s = comp.as_os_str().to_string_lossy();
        if IGNORE_VCS.contains(&s.as_ref()) {
            // Include `.git` itself and its direct children; skip
            // anything deeper.
            return i + 2 < comps.len();
        }
        if extra.iter().any(|d| d == s.as_ref()) {
            return true;
        }
    }
    false
}

pub(crate) fn load_watchman_config_ignores(root: &std::path::Path) -> Vec<String> {
    let path = root.join(".watchmanconfig");
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    let parsed: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(?e, path = ?path, "ignoring malformed .watchmanconfig");
            return Vec::new();
        }
    };
    parsed
        .get("ignore_dirs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
