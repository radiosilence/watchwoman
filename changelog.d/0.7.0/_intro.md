0.6.0 claimed memory tracked the daemon's live working set.  It did
not.  Every root ever watched leaked its whole file index, and the two
things that should have caught it were themselves broken: `status`
reported RSS, which goes down when the kernel compresses a leak away,
and the allocator purge had never once executed.  A daemon up for
thirteen days with zero roots watched held 4.4 GB.

The fix is three unrelated bugs deep, so the release also adds the
reporting that would have surfaced it on day one — footprint instead
of RSS, and allocator live-versus-mapped.

### Fixed

- **Every watched root leaked its entire file index for the life of
  the daemon** ([#23](https://github.com/radiosilence/watchwoman/pull/23)).
  The watcher, trigger and subscription tasks each held
  a strong `Arc<Root>` while awaiting a channel whose sender lived
  inside that same `Root`.  `Root::drop` is what sends the shutdown
  signal, so the tasks waited for a message that could not be sent
  until they let go: a reference cycle.  `watch-del`, `watch-del-all`
  and GC reaping all removed the root from the registry and freed
  nothing.  A daemon that had watched and released a few dozen large
  worktrees sat on multiple GB.  All three now hold `Weak<Root>` and
  upgrade per event.
- The watcher's blocking thread is now released through a channel
  instead of `JoinHandle::abort`.  A `spawn_blocking` task that has
  already started never observes cancellation, so every root also
  leaked one parked OS thread and one live fsevents/inotify
  registration.
- `arena.<all>.purge` never ran.  jemalloc spells "every arena" as
  `4096`; the sentinel was `u32::MAX - 1`, so `mallctl` returned
  `ENOENT` and the purge silently did nothing on every platform since
  the day it was added in 0.5.2.  A unit test now asserts the key
  resolves, because a wrong value fails silently rather than loudly.
- The known-gaps list claimed the microsecond/fractional time fields
  and the capability-advertisement sync were still outstanding; both
  shipped in 0.5.2
  ([#28](https://github.com/radiosilence/watchwoman/pull/28)).  The
  two real gaps are now tracked as
  [#26](https://github.com/radiosilence/watchwoman/issues/26) and
  [#27](https://github.com/radiosilence/watchwoman/issues/27).

### Changed

- `status` reports **physical footprint** as its headline memory
  number, with RSS in brackets.  RSS counts only what is resident, so
  an idle daemon holding gigabytes of compressed or swapped heap
  reported single-digit MB while Activity Monitor showed GB — the
  reporting that let the leak above hide.  `footprint_bytes` is new on
  both the top-level status object and its `memory` breakdown;
  `rss_bytes` is unchanged for existing clients.  macOS reads
  `phys_footprint` from `TASK_VM_INFO`, Linux sums `VmRSS` + `VmSwap`.
- `status` reports allocator accounting: `allocator`,
  `allocator_allocated_bytes`, `allocator_active_bytes` and
  `allocator_mapped_bytes`.  Live-versus-mapped is what separates "we
  are still using this" from "the allocator has not handed it back" —
  the distinction that made the leak diagnosable, and it needed
  `tikv-jemalloc-sys`' `stats` feature turned on.
- `unaccounted_bytes` is now measured against footprint rather than
  RSS, so compressed pages count against it.
- Declared MSRV corrected 1.82 → **1.88**
  ([#25](https://github.com/radiosilence/watchwoman/pull/25)).  1.82
  hadn't built since the `hashbrown` 0.17 bump — its cargo can't even
  parse hashbrown's manifest — so the number was a promise nothing
  kept.  The floor is `ignore` 0.4.32; the rest of the tree needs
  1.85.  CI now checks it on every run, alongside refreshed action
  pins (`checkout` v7, `sccache-action` v0.0.11, `action-gh-release`
  v3).

### Dependencies

- Bumped several dependencies past semver
  ([#21](https://github.com/radiosilence/watchwoman/pull/21)):
  `notify` 7 → 8, `nix`
  0.29 → 0.31, `directories` 5 → 6, `hashbrown` 0.15 → 0.17,
  `allocator-api2` 0.2 → 0.4, `sha1` 0.10 → 0.11.  Pulls in the
  refreshed `digest`/`crypto-common` stack (now `hybrid-array`-based,
  with `generic-array` dropped from the closure), `notify-types` 2.x,
  and `dirs-sys` 0.5.  No source changes were required: the arena
  file-index in `Tree` keeps working with `hashbrown` 0.17 unchanged
  because we hand it a `&Bump` allocator via bumpalo's
  `allocator-api2` shim (which still pins the v0.2 line), and the
  `nix`/`directories`/`notify` surface we touch is API-stable across
  these bumps.  `allocator-api2` and `directories` are still declared
  workspace pins for forward-compat but neither is imported in
  watchwoman sources today — they're inert leftovers from the 0.6.0
  arena refactor, kept in lockstep so a future use site doesn't
  silently regress them.
- Two in-semver refreshes via `cargo update` (Cargo.lock-only,
  [#20](https://github.com/radiosilence/watchwoman/pull/20),
  [#24](https://github.com/radiosilence/watchwoman/pull/24)).  The
  second drops 17 crates from the tree: `getrandom` no longer pulls
  the `wit-bindgen` / `wasm-encoder` / `wasmparser` WASI toolchain,
  which was never reachable from a build targeting unix.
