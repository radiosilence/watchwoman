### Fixed

- `status` lied about memory usage.  The reported `rss_bytes` came
  from `getrusage(RUSAGE_SELF).ru_maxrss`, which is the **peak** RSS
  the process has ever reached — monotonic, never decreases.  After
  the daemon's initial scan briefly hit 8 GB on a monorepo, every
  subsequent `status` reported 8 GB rss even after `watch-del-all`
  freed everything; ops saw a number that didn't match `ps` and
  assumed a leak.  We now read live RSS via `task_info` on macOS and
  `/proc/self/statm` on Linux; CPU times still come from `getrusage`
  where the cumulative-counter semantics are exactly what we want.
- `list-capabilities` now advertises every command the dispatcher
  actually handles — previously `get-log`, `global-log-level`, the
  `debug-*` slew, and `field-content.sha1hex` were implemented but
  not listed.  Clients doing `required` capability probes would
  spuriously miss.
- Also advertises the active watcher backend (`watcher-fsevents` on
  macOS, `watcher-inotify` on Linux).
- `watch-project` now blocks until the kernel-side
  inotify/fsevents registration is actually live before acking the
  client.  Previously it returned as soon as the watcher task was
  spawned, which left a small window where a fast `write()` from
  the same caller could land before the watch was attached and
  generate no event — surfaced as a 10–15 % flake on the trigger
  integration test under load.

### Build

- jemalloc is now gated to macOS and Linux/glibc only, dropping
  Linux/musl from the dependency.  `tikv-jemalloc-sys`' bundled
  jemalloc fails to compile under `musl-gcc` on aarch64 (C11
  stdatomic identifiers go missing through the wrapper), and
  amd64-musl was building only by GCC-default luck.  musl's own
  malloc returns pages aggressively enough that the loss is
  acceptable; the daemon falls back to the system allocator with
  `daemon::alloc::purge` no-op'd cleanly.

### Added

- Field support for `mtime_us`, `mtime_f`, `ctime_us`, `ctime_f` —
  microsecond and fractional-second variants of the time fields we
  already carry at nanosecond precision.  Pure derivations; no
  memory growth.

### Changed

- Stale-watch reaper threshold: 14 days → **1 hour**, with a new
  `WATCHWOMAN_STALE_IDLE_SECS` env knob to override (`0` disables the
  stale path entirely; dead reaping always runs).  Watches are
  ephemeral — when `jest --watch`, `watchman-wait`, or a metro server
  exits without `watch-del`, the root sat around for two weeks; an
  hour gives a long lunch's grace before reclaiming.  Active roots
  (any subscription or trigger) are still immune regardless of idle
  time.
- jemalloc is now the global allocator on macOS and Linux.  After
  every `watch-del`, `watch-del-all`, and GC reap we call
  `arena.<MALLCTL_ARENAS_ALL>.purge` so jemalloc actually
  `madvise(MADV_DONTNEED)`s the freed file-tree pages back to the
  kernel.  The system allocator on macOS hangs onto freed pages
  indefinitely, which is why dropping 1.7 M file entries didn't move
  RSS even slightly; with this change, RSS now tracks the daemon's
  live working set.
- Follow-up: arena-per-root is tracked in #13 — the larger
  architectural change for guaranteed reclaim under fragmentation.
