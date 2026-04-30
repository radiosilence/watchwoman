//! Allocator helpers — currently a single entry point: [`purge`].
//!
//! Why this exists: dropping a [`Root`](super::root::Root) frees its
//! file index back to the heap, but the allocator (jemalloc on the
//! platforms we ship to, and worse on macOS' system allocator) holds
//! the freed pages on its dirty list rather than returning them to
//! the kernel.  RSS stays stuck at the high-water mark, which is
//! exactly what made `watch-del-all` look broken.
//!
//! Calling `arena.<MALLCTL_ARENAS_ALL>.purge` after a teardown asks
//! jemalloc to actually `madvise(MADV_DONTNEED)` (or the macOS
//! equivalent) the dirty pages, which lets the kernel reclaim them
//! and `ps`/`top` reflect the drop.  No-op on non-jemalloc builds.

/// Ask the allocator to release dirty pages back to the OS.  Best-
/// effort: errors are swallowed because purging is an optimisation,
/// not a correctness requirement, and we don't want a daemon to
/// crash on a transient mallctl failure.
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
pub fn purge() {
    // SAFETY: every call below is a self-contained mallctl invocation
    // with byte-string keys.  The lengths and pointers are valid for
    // the duration of the call; jemalloc copies what it needs.

    // Bump the stats epoch first so any subsequent `status` read sees
    // a refreshed view.  Required input is a u64 == 1.
    let mut epoch_in: u64 = 1;
    unsafe {
        tikv_jemalloc_sys::mallctl(
            c"epoch".as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut epoch_in as *mut u64 as *mut _,
            std::mem::size_of::<u64>(),
        );
    }

    // The all-arenas sentinel — jemalloc treats this index as "every
    // arena".  Defined as `MALLCTL_ARENAS_ALL` in <jemalloc/jemalloc.h>;
    // we mirror the literal here so we don't need a build-time probe.
    const MALLCTL_ARENAS_ALL: u32 = u32::MAX - 1;
    let key = format!("arena.{MALLCTL_ARENAS_ALL}.purge\0");
    unsafe {
        tikv_jemalloc_sys::mallctl(
            key.as_ptr().cast(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        );
    }
}

/// No-op fallback for platforms where jemalloc isn't linked in.
#[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
pub fn purge() {}
