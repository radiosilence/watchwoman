//! In-memory file tree for a single watched root.
//!
//! The tree is keyed by the path relative to the root. Deleted files
//! leave a tombstone (`exists: false`) so `since` queries can still
//! surface the deletion to clients that missed the tick. Tombstones
//! are pruned aggressively by the GC once no cursor still cares —
//! see [`Tree::prune_tombstones_before`].  Live and tombstone counts
//! are maintained as the tree mutates so `status` can report them in
//! O(1).

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
    pub uid: u32,
    pub gid: u32,
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
        let (mode, uid, gid, ino, dev, nlink) = (
            md.mode(),
            md.uid(),
            md.gid(),
            md.ino(),
            md.dev(),
            md.nlink(),
        );
        #[cfg(not(unix))]
        let (mode, uid, gid, ino, dev, nlink) = (0, 0, 0, 0, 0, 0);

        Self {
            exists: true,
            kind,
            size: md.len(),
            mode,
            uid,
            gid,
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
    /// `exists == true` entries. Invariant: `live + tombstones == entries.len()`.
    live: usize,
    tombstones: usize,
    /// Running estimate of bytes held by the tree's heap allocations
    /// (PathBuf keys + symlink targets).  Not exact — doesn't account
    /// for BTreeMap node overhead or allocator slop — but enough to
    /// show users whether a 10 GB RSS is our data or fragmentation.
    key_bytes: usize,
    symlink_bytes: usize,
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

    /// Count of entries with `exists == true`.
    pub fn live_count(&self) -> usize {
        self.live
    }

    /// Count of tombstones (`exists == false`) still carried for the
    /// benefit of `since` queries.
    pub fn tombstone_count(&self) -> usize {
        self.tombstones
    }

    /// Bytes consumed by heap allocations we own and can estimate:
    /// path keys and symlink targets. Excludes the FileEntry struct
    /// itself (caller multiplies by `ENTRY_SIZE`) and allocator slop.
    pub fn heap_string_bytes(&self) -> usize {
        self.key_bytes + self.symlink_bytes
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Path, &FileEntry)> {
        self.entries.iter().map(|(k, v)| (k.as_path(), v))
    }

    pub fn get(&self, rel: &Path) -> Option<&FileEntry> {
        self.entries.get(rel)
    }

    /// Insert or replace an entry.  Counters are kept in sync by
    /// comparing the old entry (if any) to the new one.
    pub fn insert(&mut self, rel: PathBuf, entry: FileEntry) {
        let key_bytes = rel.as_os_str().len();
        let sym_bytes = entry.symlink_target.as_ref().map(String::len).unwrap_or(0);
        let new_exists = entry.exists;
        if let Some(old) = self.entries.insert(rel, entry) {
            self.symlink_bytes = self
                .symlink_bytes
                .saturating_sub(old.symlink_target.as_ref().map(String::len).unwrap_or(0))
                .saturating_add(sym_bytes);
            if old.exists != new_exists {
                if new_exists {
                    self.live += 1;
                    self.tombstones = self.tombstones.saturating_sub(1);
                } else {
                    self.tombstones += 1;
                    self.live = self.live.saturating_sub(1);
                }
            }
        } else {
            self.key_bytes = self.key_bytes.saturating_add(key_bytes);
            self.symlink_bytes = self.symlink_bytes.saturating_add(sym_bytes);
            if new_exists {
                self.live += 1;
            } else {
                self.tombstones += 1;
            }
        }
    }

    /// Flip an entry to a tombstone at `tick`.  No-op if the path is
    /// unknown.  Counter-safe where the old closure-based API wasn't.
    pub fn mark_gone(&mut self, rel: &Path, tick: u64) {
        let Some(e) = self.entries.get_mut(rel) else {
            return;
        };
        if e.exists {
            self.live = self.live.saturating_sub(1);
            self.tombstones += 1;
        }
        e.mark_gone(tick);
    }

    pub fn contains(&self, rel: &Path) -> bool {
        self.entries.contains_key(rel)
    }

    /// Drop tombstones whose `oclock` is at or below `watermark` —
    /// i.e. no outstanding cursor still needs them.  Returns the
    /// number of entries removed.  Cheap when there are no tombstones.
    pub fn prune_tombstones_before(&mut self, watermark: u64) -> usize {
        if self.tombstones == 0 {
            return 0;
        }
        let before_entries = self.entries.len();
        let mut freed_keys: usize = 0;
        let mut freed_sym: usize = 0;
        self.entries.retain(|path, e| {
            let keep = e.exists || e.oclock > watermark;
            if !keep {
                freed_keys += path.as_os_str().len();
                freed_sym += e.symlink_target.as_ref().map(String::len).unwrap_or(0);
            }
            keep
        });
        let removed = before_entries - self.entries.len();
        self.tombstones = self.tombstones.saturating_sub(removed);
        self.key_bytes = self.key_bytes.saturating_sub(freed_keys);
        self.symlink_bytes = self.symlink_bytes.saturating_sub(freed_sym);
        removed
    }
}

/// In-memory size of a [`FileEntry`] struct — used by `status` to
/// estimate tree footprint without walking every entry.
pub const ENTRY_SIZE: usize = std::mem::size_of::<FileEntry>();

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(exists: bool, oclock: u64, sym: Option<&str>) -> FileEntry {
        FileEntry {
            exists,
            kind: FileKind::File,
            size: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            mtime_ns: 0,
            ctime_ns: 0,
            mtime_ms: 0,
            ctime_ms: 0,
            ino: 0,
            dev: 0,
            nlink: 1,
            symlink_target: sym.map(str::to_owned),
            cclock: 1,
            oclock,
            is_new: false,
        }
    }

    #[test]
    fn insert_and_mark_gone_track_counts() {
        let mut t = Tree::new();
        t.insert(PathBuf::from("a"), entry(true, 1, None));
        t.insert(PathBuf::from("b"), entry(true, 2, None));
        assert_eq!((t.live_count(), t.tombstone_count()), (2, 0));

        t.mark_gone(Path::new("a"), 3);
        assert_eq!((t.live_count(), t.tombstone_count()), (1, 1));

        // Marking a tombstone again is a no-op for counters.
        t.mark_gone(Path::new("a"), 4);
        assert_eq!((t.live_count(), t.tombstone_count()), (1, 1));

        // Unknown path: silent no-op.
        t.mark_gone(Path::new("nope"), 5);
        assert_eq!((t.live_count(), t.tombstone_count()), (1, 1));
    }

    #[test]
    fn resurrect_flips_counters_back() {
        let mut t = Tree::new();
        t.insert(PathBuf::from("a"), entry(true, 1, None));
        t.mark_gone(Path::new("a"), 2);
        assert_eq!((t.live_count(), t.tombstone_count()), (0, 1));

        // Re-inserting with exists=true flips it back to live.
        t.insert(PathBuf::from("a"), entry(true, 3, None));
        assert_eq!((t.live_count(), t.tombstone_count()), (1, 0));
    }

    #[test]
    fn prune_drops_tombstones_at_or_below_watermark() {
        let mut t = Tree::new();
        t.insert(PathBuf::from("live"), entry(true, 1, None));
        t.insert(PathBuf::from("gone-old"), entry(false, 5, None));
        t.insert(PathBuf::from("gone-new"), entry(false, 20, None));
        assert_eq!((t.live_count(), t.tombstone_count()), (1, 2));

        // Watermark 10: drops oclock=5, keeps oclock=20 and the live one.
        let removed = t.prune_tombstones_before(10);
        assert_eq!(removed, 1);
        assert_eq!((t.live_count(), t.tombstone_count()), (1, 1));
        assert!(t.contains(Path::new("live")));
        assert!(!t.contains(Path::new("gone-old")));
        assert!(t.contains(Path::new("gone-new")));
    }

    #[test]
    fn prune_is_noop_when_no_tombstones() {
        let mut t = Tree::new();
        t.insert(PathBuf::from("a"), entry(true, 1, None));
        assert_eq!(t.prune_tombstones_before(u64::MAX), 0);
        assert_eq!(t.live_count(), 1);
    }

    #[test]
    fn heap_bytes_track_inserts_and_prunes() {
        let mut t = Tree::new();
        t.insert(
            PathBuf::from("aaa"),
            entry(true, 1, Some("link-target-bytes")),
        );
        let after_insert = t.heap_string_bytes();
        assert_eq!(after_insert, "aaa".len() + "link-target-bytes".len());

        t.mark_gone(Path::new("aaa"), 2);
        // mark_gone doesn't free the key — still tombstoned.
        assert_eq!(t.heap_string_bytes(), after_insert);

        t.prune_tombstones_before(u64::MAX);
        assert_eq!(t.heap_string_bytes(), 0);
    }
}
