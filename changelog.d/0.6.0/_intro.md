The headline is structural memory reclaim: per-root file index moves
from a globally-allocated `BTreeMap` to a per-root bumpalo arena.
Dropping a root now `munmap`s a single contiguous range instead of
hoping the allocator cooperates with a purge.  Together with 0.5.2's
jemalloc work, RSS now tracks the daemon's actual working set
through arbitrary watch churn.

### Changed

- Per-root file index now lives in a [bumpalo](https://docs.rs/bumpalo)
  arena: path keys, symlink targets, the HashMap's backing array, and
  every `FileEntry` payload all allocate from one contiguous range
  per root.  Drop the root → drop the arena → bumpalo `munmap`s the
  range as a single block, returning memory to the OS without depending
  on the global allocator's cooperation.  This is the architectural
  follow-up to 0.5.2's jemalloc purge: the purge cleaned freed pages,
  but couldn't help when allocations from one root were interleaved
  on the same page with allocations from a still-live root —
  fragmentation defeated the reclaim.  An arena ensures the per-root
  pages are unshared and therefore actually returnable.  Closes #13.
- `BTreeMap` in the tree replaced with `hashbrown::HashMap` so we can
  store every entry inside the arena via the `allocator-api2`
  allocator interface.  Iteration order is no longer guaranteed —
  callers that need a stable order must sort the result themselves.
- `status` now reports the exact arena-allocated bytes per root
  (`tree_bytes_est`); the previous estimator (entry-count × const +
  string-length tally + BTreeMap fudge factor) is gone.

### Added

- `status` emits `version` (real watchwoman semver) and
  `compat_version` (the watchman date-stamp we hand back to client
  compat probes) alongside the existing fields.  The CLI render's
  `watchwoman {version}  (pid …)` slot was reading this already; the
  daemon just hadn't been populating it.  Output now reads
  `watchwoman 0.6.0  (pid …, up …)` instead of a blank slot.
