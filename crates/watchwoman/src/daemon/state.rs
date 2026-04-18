//! Global daemon state shared across connections.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Context;
use dashmap::DashMap;
use tokio::sync::{mpsc, Notify};

use super::root::{Root, WatcherCommand};
use super::watcher;

pub struct DaemonState {
    pub sock_path: PathBuf,
    pub roots: DashMap<PathBuf, Arc<Root>>,
    pub shutdown: Arc<Notify>,
    pub shutting_down: AtomicBool,
    root_counter: AtomicU64,
}

impl DaemonState {
    pub fn new(sock_path: PathBuf) -> Self {
        Self {
            sock_path,
            roots: DashMap::new(),
            shutdown: Arc::new(Notify::new()),
            shutting_down: AtomicBool::new(false),
            root_counter: AtomicU64::new(0),
        }
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
        self.roots.get(path).map(|r| r.clone())
    }

    pub fn list_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|e| e.key().clone()).collect()
    }

    /// Register a new root and spawn its watcher.  Idempotent — returning
    /// the existing [`Root`] if one is already installed.
    pub fn register_root(self: &Arc<Self>, path: PathBuf) -> anyhow::Result<Arc<Root>> {
        if let Some(existing) = self.roots.get(&path) {
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
        let root = Arc::new(Root::new(path.clone(), root_number, root_counter, cmd_tx));

        // Seed the tree synchronously so the first query from the caller
        // sees a populated root — otherwise tools that watch-and-query in
        // the same breath (jest, metro, hg) race the initial scan.
        let entries = watcher::initial_scan(&path);
        root.seed(entries);

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
