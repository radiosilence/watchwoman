//! Allocator helpers: [`purge`] to hand freed pages back to the OS,
//! and [`stats`] to report what the allocator is still holding.
//!
//! Why `purge` exists: dropping a [`Root`](super::root::Root) frees
//! its file index back to the heap, but the allocator can sit on those
//! pages rather than return them to the kernel.  Memory stays stuck at
//! the high-water mark, which is exactly what made `watch-del-all`
//! look broken.  `arena.<all>.purge` asks jemalloc to `madvise` the
//! dirty pages away so the drop shows up in `status` and in `top`.
//!
//! The sentinel below is worth a word.  jemalloc spells "every arena"
//! as the literal `4096` (`MALLCTL_ARENAS_ALL` in
//! `<jemalloc/jemalloc.h>`) — it is an index one past the maximum
//! arena count, not a saturated integer.  Getting it wrong doesn't
//! fail loudly: `mallctl` just returns `ENOENT` for the unknown key
//! and the purge silently does nothing.

/// A snapshot of what the allocator is holding, in bytes.  `None` for
/// a field the current allocator can't report.
///
/// The pair that matters when triaging a fat daemon is `allocated` vs
/// `mapped`: memory the program is still using, against memory the
/// allocator has taken from the kernel.  A large `allocated` with no
/// roots watched is a leak in watchwoman; a small `allocated` under a
/// large `mapped` is the allocator hoarding, which is [`purge`]'s job.
#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    /// Bytes handed out to live allocations.
    pub allocated: Option<u64>,
    /// Bytes in pages the allocator considers in use, including its
    /// own slack within those pages.
    pub active: Option<u64>,
    /// Bytes the allocator has mapped from the kernel in total.
    pub mapped: Option<u64>,
    /// Which allocator produced these numbers.
    pub allocator: &'static str,
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod imp {
    pub const NAME: &str = "jemalloc";

    /// jemalloc's "every arena" index.  See the module docs — this is
    /// `4096`, and a wrong value degrades to a silent no-op.
    const MALLCTL_ARENAS_ALL: u32 = 4096;

    /// Ask jemalloc to release dirty pages back to the OS.  Best-
    /// effort: errors are swallowed because purging is an
    /// optimisation, not a correctness requirement, and we don't want
    /// a daemon to crash on a transient mallctl failure.
    pub fn purge() {
        refresh_epoch();
        let key = format!("arena.{MALLCTL_ARENAS_ALL}.purge\0");
        // SAFETY: a self-contained mallctl invocation with a
        // NUL-terminated key valid for the duration of the call.
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

    pub fn stats() -> super::Stats {
        // jemalloc's stats counters are cached behind an epoch; without
        // advancing it every read returns the values from process
        // start.
        refresh_epoch();
        super::Stats {
            allocated: read_size(c"stats.allocated"),
            active: read_size(c"stats.active"),
            mapped: read_size(c"stats.mapped"),
            allocator: NAME,
        }
    }

    fn refresh_epoch() {
        let mut epoch: u64 = 1;
        // SAFETY: `epoch` is a live u64 for the duration of the call
        // and the declared length matches it exactly.
        unsafe {
            tikv_jemalloc_sys::mallctl(
                c"epoch".as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut epoch as *mut u64 as *mut _,
                std::mem::size_of::<u64>(),
            );
        }
    }

    /// Read one `size_t`-valued mallctl.  `None` if the key is absent,
    /// which is what a jemalloc built without `--enable-stats` gives
    /// us for the `stats.*` tree.
    fn read_size(key: &std::ffi::CStr) -> Option<u64> {
        let mut out: usize = 0;
        let mut len = std::mem::size_of::<usize>();
        // SAFETY: `out` and `len` are live for the call and `len`
        // describes `out` exactly; jemalloc writes at most that much.
        let rc = unsafe {
            tikv_jemalloc_sys::mallctl(
                key.as_ptr(),
                &mut out as *mut usize as *mut _,
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (rc == 0).then_some(out as u64)
    }

    #[cfg(test)]
    mod tests {
        /// The sentinel is a bare literal with no compile-time check
        /// behind it, and a wrong one fails silently.  `mallctl`
        /// returning 0 rather than ENOENT is the only proof the key
        /// exists in the jemalloc we actually linked.
        #[test]
        fn arenas_all_sentinel_is_a_real_mallctl_key() {
            let key = format!("arena.{}.purge\0", super::MALLCTL_ARENAS_ALL);
            let rc = unsafe {
                tikv_jemalloc_sys::mallctl(
                    key.as_ptr().cast(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                )
            };
            assert_eq!(rc, 0, "arena.<all>.purge rejected: mallctl rc={rc}");
        }

        #[test]
        fn stats_are_compiled_in() {
            assert!(
                super::stats().allocated.is_some(),
                "jemalloc built without --enable-stats; status can't report allocator usage"
            );
        }
    }
}

#[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
mod imp {
    pub const NAME: &str = "system";

    pub fn purge() {}

    pub fn stats() -> super::Stats {
        super::Stats {
            allocator: NAME,
            ..Default::default()
        }
    }
}

/// Ask the allocator to release freed pages back to the OS.
pub fn purge() {
    imp::purge();
}

/// Current allocator accounting.  Fields the platform can't answer are
/// `None`.
pub fn stats() -> Stats {
    imp::stats()
}
