use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use tempfile::TempDir;

/// Throwaway project directory.  Always looks like a watchman-eligible
/// project (contains `.git/`) so `watch-project` happily roots at it.
pub struct Scratch {
    dir: TempDir,
}

impl Scratch {
    pub fn new() -> anyhow::Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("watchwoman-scratch-")
            .tempdir()
            .context("creating scratch dir")?;
        fs::create_dir_all(dir.path().join(".git"))?;
        Ok(Self { dir })
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn write(&self, rel: &str, contents: &[u8]) -> anyhow::Result<PathBuf> {
        let p = self.dir.path().join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&p, contents)?;
        Ok(p)
    }

    pub fn mkdir(&self, rel: &str) -> anyhow::Result<PathBuf> {
        let p = self.dir.path().join(rel);
        fs::create_dir_all(&p)?;
        Ok(p)
    }
}
