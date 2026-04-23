//! Global daemon state shared across connections.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::sync::{mpsc, Notify};

use super::root::{Root, WatcherCommand};
use super::watcher;

/// A GC-initiated watch-del, kept for the `status` command so
/// operators can see what disappeared and why without tailing logs.
#[derive(Debug, Clone)]
pub struct ReapEvent {
    pub path: PathBuf,
    pub reason: ReapReason,
    pub at_unix: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum ReapReason {
    /// The root's directory was missing from disk on >=2 consecutive
    /// GC ticks — probably deleted or on an unmounted volume.
    Dead,
    /// No subscriptions, no triggers, no commands for a long time.
    Stale,
}

impl ReapReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ReapReason::Dead => "dead",
            ReapReason::Stale => "stale",
        }
    }
}

/// Keep the last N reap events; older ones fall off the front.  The
/// `status` report surfaces these so `watchman status` doubles as a
/// post-mortem for "where did my watch go?".
const REAP_LOG_CAPACITY: usize = 64;

pub struct DaemonState {
    pub sock_path: PathBuf,
    pub roots: DashMap<PathBuf, Arc<Root>>,
    pub shutdown: Arc<Notify>,
    pub shutting_down: AtomicBool,
    /// Monotonic clock anchor for `uptime_seconds` in the status
    /// report — survives wall-clock jumps, suspends, etc.
    pub started_at: Instant,
    pub started_at_unix: u64,
    reap_log: RwLock<VecDeque<ReapEvent>>,
    root_counter: AtomicU64,
}

impl DaemonState {
    pub fn new(sock_path: PathBuf) -> Self {
        let started_at_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            sock_path,
            roots: DashMap::new(),
            shutdown: Arc::new(Notify::new()),
            shutting_down: AtomicBool::new(false),
            started_at: Instant::now(),
            started_at_unix,
            reap_log: RwLock::new(VecDeque::with_capacity(REAP_LOG_CAPACITY)),
            root_counter: AtomicU64::new(0),
        }
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Record a GC reap for the status report's history section.
    pub fn log_reap(&self, event: ReapEvent) {
        let mut log = self.reap_log.write();
        if log.len() == REAP_LOG_CAPACITY {
            log.pop_front();
        }
        log.push_back(event);
    }

    pub fn reap_log(&self) -> Vec<ReapEvent> {
        self.reap_log.read().iter().cloned().collect()
    }

    pub fn request_shutdown(&self) {
        if !self.shutting_down.swap(true, Ordering::AcqRel) {
            self.shutdown.notify_waiters();
        }
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Acquire)
    }

    pub fn root(&self, path: &Path) -> Option<Arc<Root>> {
        let root = self.roots.get(path).map(|r| r.clone())?;
        // Every command that reaches a known root resets the staleness
        // timer — the GC only reaps watches nobody's touched.
        root.touch();
        Some(root)
    }

    pub fn list_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|e| e.key().clone()).collect()
    }

    /// Register a new root and spawn its watcher.  Idempotent — returning
    /// the existing [`Root`] if one is already installed.
    pub fn register_root(self: &Arc<Self>, path: PathBuf) -> anyhow::Result<Arc<Root>> {
        if let Some(existing) = self.roots.get(&path) {
            // Re-watching counts as activity; resets the stale timer
            // for long-lived tools that re-`watch-project` on every run.
            existing.touch();
            return Ok(existing.clone());
        }

        let metadata = std::fs::metadata(&path)
            .with_context(|| format!("resolving metadata for {}", path.display()))?;
        if !metadata.is_dir() {
            anyhow::bail!("watch root {} is not a directory", path.display());
        }

        let root_number = self.root_counter.fetch_add(1, Ordering::AcqRel) + 1;
        let root_counter = Arc::new(AtomicU64::new(root_number));
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<WatcherCommand>();
        let state_dir = self.sock_path.parent().map(|p| p.to_path_buf());
        let ignore_dirs = watcher::load_watchman_config_ignores(&path);
        let root = Arc::new(Root::new(
            path.clone(),
            root_number,
            root_counter,
            cmd_tx,
            state_dir,
            ignore_dirs.clone(),
        ));

        // Seed the tree synchronously so the first query from the caller
        // sees a populated root — otherwise tools that watch-and-query in
        // the same breath (jest, metro, hg) race the initial scan.
        let entries = watcher::initial_scan(&path, &ignore_dirs);
        root.seed(entries);

        // Rehydrate durable triggers from the last run and restart
        // their fork-and-exec loops so a daemon restart is invisible.
        root.load_persisted_triggers();
        for trigger in root.list_triggers() {
            crate::commands::trigger::spawn_trigger_loop_ext(
                root.clone(),
                root.path.clone(),
                trigger,
            );
        }

        self.roots.insert(path.clone(), root.clone());

        watcher::spawn(root.clone(), cmd_rx);

        Ok(root)
    }

    pub fn unregister_root(&self, path: &Path) -> bool {
        self.roots.remove(path).is_some()
    }

    pub fn drain_roots(&self) -> Vec<PathBuf> {
        let mut removed = Vec::new();
        let paths: Vec<PathBuf> = self.roots.iter().map(|e| e.key().clone()).collect();
        for p in paths {
            if self.roots.remove(&p).is_some() {
                removed.push(p);
            }
        }
        removed
    }
}
