use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use tempfile::TempDir;

/// Throwaway project directory.  Always looks like a watchman-eligible
/// project (contains `.git/`) so `watch-project` happily roots at it.
/// The tempdir itself keeps its system path so `drop` cleans up, but
/// [`Self::path`] returns the canonical path (e.g. `/private/var/...`
/// on macOS) since that's what the daemon sees and reports.
pub struct Scratch {
    dir: TempDir,
    canonical: PathBuf,
}

impl Scratch {
    pub fn new() -> anyhow::Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("watchwoman-scratch-")
            .tempdir()
            .context("creating scratch dir")?;
        fs::create_dir_all(dir.path().join(".git"))?;
        let canonical = std::fs::canonicalize(dir.path()).context("canonicalising scratch")?;
        Ok(Self { dir, canonical })
    }

    pub fn path(&self) -> &Path {
        &self.canonical
    }

    pub fn raw_path(&self) -> &Path {
        self.dir.path()
    }

    pub fn write(&self, rel: &str, contents: &[u8]) -> anyhow::Result<PathBuf> {
        let p = self.canonical.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&p, contents)?;
        Ok(p)
    }

    pub fn mkdir(&self, rel: &str) -> anyhow::Result<PathBuf> {
        let p = self.canonical.join(rel);
        fs::create_dir_all(&p)?;
        Ok(p)
    }
}
