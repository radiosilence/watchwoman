//! In-memory file tree for a single watched root.
//!
//! The tree is keyed by the path relative to the root. Absent files
//! keep their entry (`exists: false`) so `since` queries surface
//! deletions. Memory footprint is roughly one [`FileEntry`] per
//! discovered path — no stat cache beyond that.

use std::collections::BTreeMap;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub exists: bool,
    pub kind: FileKind,
    pub size: u64,
    pub mode: u32,
    pub mtime_ns: i128,
    pub ctime_ns: i128,
    pub mtime_ms: i64,
    pub ctime_ms: i64,
    pub ino: u64,
    pub dev: u64,
    pub nlink: u64,
    pub symlink_target: Option<String>,
    pub cclock: u64,
    pub oclock: u64,
    pub is_new: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Dir,
    Symlink,
    Block,
    Char,
    Fifo,
    Socket,
    Unknown,
}

impl FileKind {
    pub fn as_wire_str(&self) -> &'static str {
        match self {
            FileKind::File => "f",
            FileKind::Dir => "d",
            FileKind::Symlink => "l",
            FileKind::Block => "b",
            FileKind::Char => "c",
            FileKind::Fifo => "p",
            FileKind::Socket => "s",
            FileKind::Unknown => "?",
        }
    }
}

impl FileEntry {
    pub fn from_metadata(md: &Metadata, symlink_target: Option<String>, clock_tick: u64) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        let kind = if md.is_symlink() {
            FileKind::Symlink
        } else if md.is_dir() {
            FileKind::Dir
        } else if md.is_file() {
            FileKind::File
        } else {
            classify_unix(md)
        };

        let mtime_ns = metadata_time_ns(md.modified().ok());
        let ctime_ns = metadata_time_ns(md.created().ok().or_else(|| md.modified().ok()));

        #[cfg(unix)]
        let (mode, ino, dev, nlink) = (md.mode(), md.ino(), md.dev(), md.nlink());
        #[cfg(not(unix))]
        let (mode, ino, dev, nlink) = (0, 0, 0, 0);

        Self {
            exists: true,
            kind,
            size: md.len(),
            mode,
            mtime_ns,
            ctime_ns,
            mtime_ms: (mtime_ns / 1_000_000) as i64,
            ctime_ms: (ctime_ns / 1_000_000) as i64,
            ino,
            dev,
            nlink,
            symlink_target,
            cclock: clock_tick,
            oclock: clock_tick,
            is_new: true,
        }
    }

    pub fn mark_gone(&mut self, clock_tick: u64) {
        self.exists = false;
        self.oclock = clock_tick;
        self.is_new = false;
    }
}

fn metadata_time_ns(t: Option<SystemTime>) -> i128 {
    t.and_then(|ts| ts.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i128)
        .unwrap_or_default()
}

#[cfg(unix)]
fn classify_unix(md: &Metadata) -> FileKind {
    use std::os::unix::fs::FileTypeExt;
    let ft = md.file_type();
    if ft.is_block_device() {
        FileKind::Block
    } else if ft.is_char_device() {
        FileKind::Char
    } else if ft.is_fifo() {
        FileKind::Fifo
    } else if ft.is_socket() {
        FileKind::Socket
    } else {
        FileKind::Unknown
    }
}

#[cfg(not(unix))]
fn classify_unix(_md: &Metadata) -> FileKind {
    FileKind::Unknown
}

#[derive(Debug, Default)]
pub struct Tree {
    /// Relative path → entry.  BTreeMap keeps iteration deterministic for
    /// snapshot-style tests.
    entries: BTreeMap<PathBuf, FileEntry>,
}

impl Tree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Path, &FileEntry)> {
        self.entries.iter().map(|(k, v)| (k.as_path(), v))
    }

    pub fn get(&self, rel: &Path) -> Option<&FileEntry> {
        self.entries.get(rel)
    }

    pub fn insert(&mut self, rel: PathBuf, entry: FileEntry) {
        self.entries.insert(rel, entry);
    }

    pub fn update(&mut self, rel: &Path, mutate: impl FnOnce(&mut FileEntry)) {
        if let Some(e) = self.entries.get_mut(rel) {
            mutate(e);
        }
    }

    pub fn contains(&self, rel: &Path) -> bool {
        self.entries.contains_key(rel)
    }
}
