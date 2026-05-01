//! In-memory file tree for a single watched root.
//!
//! All allocations for this tree live in a per-root bumpalo arena
//! ([`Tree::arena`]): path keys, symlink targets, the HashMap's
//! backing array, and the `FileEntry` payloads (stored inline in
//! that array). When the root is dropped, the arena is dropped, and
//! bumpalo `munmap`s its chunks in one contiguous range — returning
//! memory to the OS without depending on the global allocator's
//! cooperation.  This is the architectural cousin of jemalloc's
//! `arena.<all>.purge`: that purge cleans pages we freed; this arena
//! ensures the per-root pages are *contiguous and unshared* so they
//! actually can be returned, rather than interleaved with allocations
//! from other still-live roots.
//!
//! Iteration order is no longer deterministic — we trade BTreeMap's
//! sortedness for HashMap's per-entry inline storage in the arena.
//! Callers that need a stable order must sort the result themselves.
//!
//! Deleted files leave a tombstone (`exists: false`) so `since`
//! queries can still surface the deletion to clients that missed
//! the tick.  Tombstones are pruned aggressively by the GC once no
//! cursor still cares — see [`Tree::prune_tombstones_before`].

use std::ffi::OsStr;
use std::fs::Metadata;
use std::path::Path;
use std::time::SystemTime;

use bumpalo::Bump;
use hashbrown::HashMap;

#[derive(Debug, Clone)]
pub struct FileEntry<'a> {
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
    pub symlink_target: Option<&'a str>,
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

impl<'a> FileEntry<'a> {
    pub fn from_metadata(md: &Metadata, symlink_target: Option<&'a str>, clock_tick: u64) -> Self {
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

/// Per-root file tree backed by a bumpalo arena.
///
/// # Self-referential layout
///
/// The HashMap stores `&Path` keys, `FileEntry<'a>` values, and
/// holds a `&Bump` allocator — all of which logically borrow from
/// `arena`.  Rust can't express that directly, so internally the
/// references are typed as `&'static`; externally the API only ever
/// hands out references tied to `&self`, which is sound because the
/// arena outlives the map.
///
/// # Safety invariants
///
/// 1. `arena` is heap-allocated (`Box<Bump>`) so its address is
///    stable for the lifetime of the `Tree`.
/// 2. Field declaration order is `entries` *before* `arena`. Rust
///    drops fields in declaration order, so `entries` (and every
///    reference it owns) tears down first; the arena follows.
/// 3. We never expose the `'static` references outside the impl.
///    All public methods either take `&self` and return references
///    bounded by `&self`, or take owned/borrowed inputs and copy
///    bytes into the arena.
pub struct Tree {
    // SAFETY: see the type-level docs.  These references really
    // borrow from `self.arena`.
    entries:
        HashMap<&'static Path, FileEntry<'static>, hashbrown::DefaultHashBuilder, &'static Bump>,
    arena: Box<Bump>,
    /// `exists == true` entries.  Invariant: `live + tombstones == entries.len()`.
    live: usize,
    tombstones: usize,
}

// SAFETY: `Bump`'s interior `Cell` fields make it `!Sync`, which
// transitively infects everything storing `&Bump` (the HashMap's
// allocator slot) and therefore `Tree`.  We declare `Send`/`Sync`
// manually because the daemon's locking discipline already provides
// the exclusion the auto-derived bounds would have required:
//
//   * `Send`: a `Tree` is owned through `Arc<Root>` and only ever
//     moved at construction; the `Bump` and HashMap travel together
//     so no aliasing occurs across the move.
//   * `Sync`: every external use site wraps `Tree` in
//     `parking_lot::RwLock<Tree>`.  All mutating methods take
//     `&mut self` (only callable under the write lock, which excludes
//     all readers).  Read methods (`iter`, `get`, `contains`, `len`,
//     `arena_bytes`, …) only ever read from the HashMap or call
//     `Bump::allocated_bytes`, neither of which writes to the Cells
//     that prevent the auto-trait.  Concurrent readers are therefore
//     sound under the lock.
unsafe impl Send for Tree {}
unsafe impl Sync for Tree {}

impl std::fmt::Debug for Tree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tree")
            .field("len", &self.entries.len())
            .field("live", &self.live)
            .field("tombstones", &self.tombstones)
            .field("arena_bytes", &self.arena_bytes())
            .finish()
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

impl Tree {
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    /// Build a tree pre-sized for `capacity` entries — used by the
    /// initial-scan path to avoid the geometric grow chain dirtying
    /// the arena.
    pub fn with_capacity(capacity: usize) -> Self {
        let arena = Box::new(Bump::new());
        // SAFETY: `arena` lives in this struct alongside `entries`.
        // Field drop order ensures `entries` drops first, so the
        // `&Bump` reference stored inside it can never dangle.  The
        // `'static` lifetime is a placeholder that is shrunk back
        // to `&self` lifetimes at every public API boundary.
        let arena_static: &'static Bump =
            unsafe { std::mem::transmute::<&Bump, &'static Bump>(&*arena) };
        let entries = if capacity == 0 {
            HashMap::new_in(arena_static)
        } else {
            HashMap::with_capacity_in(capacity, arena_static)
        };
        Self {
            entries,
            arena,
            live: 0,
            tombstones: 0,
        }
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

    /// Bytes the arena has handed out so far — paths, symlinks,
    /// HashMap backing array, every `FileEntry` payload.  Includes
    /// arena chunk overhead (small).  Source of truth for the
    /// per-root memory footprint reported by `status`.
    pub fn arena_bytes(&self) -> usize {
        self.arena.allocated_bytes()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Path, &FileEntry<'_>)> + '_ {
        // Covariance + auto-deref shrink the stored `'static`
        // lifetimes down to `&self`; sound because the arena outlives
        // `&self`.
        self.entries.iter().map(|(k, v)| {
            let k: &Path = k;
            let v: &FileEntry<'_> = v;
            (k, v)
        })
    }

    pub fn get(&self, rel: &Path) -> Option<&FileEntry<'_>> {
        self.entries.get(rel).map(|v| {
            let v: &FileEntry<'_> = v;
            v
        })
    }

    pub fn contains(&self, rel: &Path) -> bool {
        self.entries.contains_key(rel)
    }

    /// Insert or update an entry.  Path bytes (for new keys) and
    /// the symlink target (always, when present) are copied into
    /// the arena.
    pub fn upsert(
        &mut self,
        rel: &Path,
        md: &Metadata,
        symlink_target: Option<&str>,
        clock_tick: u64,
        is_new: bool,
        cclock: u64,
    ) {
        // If the key already exists, reuse its arena-allocated
        // bytes; otherwise allocate fresh.
        let key: &'static Path = match self.entries.get_key_value(rel) {
            Some((existing, _)) => existing,
            None => self.intern_path(rel),
        };

        let new_sym: Option<&'static str> = symlink_target.map(|s| self.intern_str(s));
        let mut entry = FileEntry::from_metadata(md, new_sym, clock_tick);
        entry.is_new = is_new;
        entry.cclock = cclock;

        match self.entries.insert(key, entry) {
            Some(old) => {
                if !old.exists {
                    self.live += 1;
                    self.tombstones = self.tombstones.saturating_sub(1);
                }
            }
            None => {
                self.live += 1;
            }
        }
    }

    /// Flip an entry to a tombstone at `tick`.  No-op if the path
    /// is unknown.  Counter-safe.
    pub fn mark_gone(&mut self, rel: &Path, tick: u64) {
        let Some(e) = self.entries.get_mut(rel) else {
            return;
        };
        if e.exists {
            self.live = self.live.saturating_sub(1);
            self.tombstones += 1;
        }
        e.exists = false;
        e.oclock = tick;
        e.is_new = false;
    }

    /// Drop tombstones whose `oclock` is at or below `watermark` —
    /// i.e. no outstanding cursor still needs them.  Returns the
    /// number of entries removed.  Cheap when there are no tombstones.
    ///
    /// Note: the freed key/symlink bytes stay in the arena (bumpalo
    /// doesn't reclaim individual allocations).  They are reclaimed
    /// when the whole `Tree` drops.
    pub fn prune_tombstones_before(&mut self, watermark: u64) -> usize {
        if self.tombstones == 0 {
            return 0;
        }
        let before = self.entries.len();
        self.entries.retain(|_, e| e.exists || e.oclock > watermark);
        let removed = before - self.entries.len();
        self.tombstones = self.tombstones.saturating_sub(removed);
        removed
    }

    fn intern_path(&self, rel: &Path) -> &'static Path {
        let bytes = self
            .arena
            .alloc_slice_copy(rel.as_os_str().as_encoded_bytes());
        // SAFETY: bytes were obtained from a valid `OsStr`'s encoding,
        // so the inverse round-trip is sound.
        let os: &OsStr = unsafe { OsStr::from_encoded_bytes_unchecked(bytes) };
        let path: &Path = Path::new(os);
        // SAFETY: extends the arena-bound lifetime to `'static` per
        // the type-level invariant.
        unsafe { std::mem::transmute::<&Path, &'static Path>(path) }
    }

    fn intern_str(&self, s: &str) -> &'static str {
        let allocated: &str = self.arena.alloc_str(s);
        // SAFETY: see [`intern_path`].
        unsafe { std::mem::transmute::<&str, &'static str>(allocated) }
    }
}

/// In-memory size of a [`FileEntry`] — used by `status` for shape-
/// only diagnostics.  The arena holds this many bytes per entry plus
/// the path bytes plus the (rare) symlink target plus HashMap slot
/// overhead.
pub const ENTRY_SIZE: usize = std::mem::size_of::<FileEntry<'static>>();

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn entry(exists: bool, oclock: u64, sym: Option<&'static str>) -> FileEntry<'static> {
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
            symlink_target: sym,
            cclock: 1,
            oclock,
            is_new: false,
        }
    }

    /// Insert `path` directly with a synthetic value — bypasses
    /// `upsert` so tests don't need a real `Metadata`.
    fn raw_insert(t: &mut Tree, path: &Path, e: FileEntry<'static>) {
        let key = t.intern_path(path);
        let new_sym = e.symlink_target.map(|s| t.intern_str(s));
        let mut e = e;
        e.symlink_target = new_sym;
        let new_exists = e.exists;
        let prev = t.entries.insert(key, e);
        match prev {
            Some(old) => {
                if old.exists != new_exists {
                    if new_exists {
                        t.live += 1;
                        t.tombstones = t.tombstones.saturating_sub(1);
                    } else {
                        t.tombstones += 1;
                        t.live = t.live.saturating_sub(1);
                    }
                }
            }
            None => {
                if new_exists {
                    t.live += 1;
                } else {
                    t.tombstones += 1;
                }
            }
        }
    }

    #[test]
    fn insert_and_mark_gone_track_counts() {
        let mut t = Tree::new();
        raw_insert(&mut t, Path::new("a"), entry(true, 1, None));
        raw_insert(&mut t, Path::new("b"), entry(true, 2, None));
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
        raw_insert(&mut t, Path::new("a"), entry(true, 1, None));
        t.mark_gone(Path::new("a"), 2);
        assert_eq!((t.live_count(), t.tombstone_count()), (0, 1));

        // Re-inserting with exists=true flips it back to live.
        raw_insert(&mut t, Path::new("a"), entry(true, 3, None));
        assert_eq!((t.live_count(), t.tombstone_count()), (1, 0));
    }

    #[test]
    fn prune_drops_tombstones_at_or_below_watermark() {
        let mut t = Tree::new();
        raw_insert(&mut t, Path::new("live"), entry(true, 1, None));
        raw_insert(&mut t, Path::new("gone-old"), entry(false, 5, None));
        raw_insert(&mut t, Path::new("gone-new"), entry(false, 20, None));
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
        raw_insert(&mut t, Path::new("a"), entry(true, 1, None));
        assert_eq!(t.prune_tombstones_before(u64::MAX), 0);
        assert_eq!(t.live_count(), 1);
    }

    #[test]
    fn arena_grows_with_inserts() {
        let mut t = Tree::new();
        let before = t.arena_bytes();
        for i in 0..32 {
            let path = format!("file-{i:03}");
            raw_insert(
                &mut t,
                Path::new(&path),
                entry(true, i as u64, Some("target")),
            );
        }
        let after = t.arena_bytes();
        assert!(
            after > before,
            "arena should have allocated bytes: before={before}, after={after}"
        );
        assert_eq!(t.live_count(), 32);
    }
}
