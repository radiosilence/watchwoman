### Added

- Tombstone pruning.  Deleted files left `exists: false` entries in
  the per-root tree forever so `since` queries could still surface
  the deletion — fine for small repos, catastrophic on a long-lived
  daemon against a monorepo with heavy branch churn (RSS of 8 GB+
  observed on a 3.6 M-file tree after 5 h).  The GC sweep now prunes
  tombstones whose `oclock` no longer matters to any named cursor on
  every 60 s tick; subscribers consume deletions live via tick events
  and don't block the watermark.  `debug-ageout` now actually runs a
  sweep instead of being a shape-only no-op.
- `status` grows a memory breakdown: per-root `live_files` /
  `tombstones` / `tree_bytes_est` columns, plus a top-level
  `memory` object reporting RSS, estimated tracked-data bytes, and
  the unaccounted remainder (allocator fragmentation / OS-held pages)
  so the 8 GB mystery becomes "1 GB data + 7 GB fragmentation" at a
  glance.

### Changed

- Build / release profile tightening.  Tokio is pulled with the six
  features we actually use (`rt-multi-thread`, `macros`, `sync`,
  `net`, `io-util`, `time`) instead of `full`; release binaries are
  now `strip = "symbols"` instead of `debuginfo`-only.  Clean release
  build goes 18.20 s → 17.14 s locally; the shipped six-binary
  payload drops from **9.7 MB → 8.3 MB** (`watchwoman`: 3.2 → 2.8 MB).
  No behaviour change; CI / test / clippy pass identically.
- CI layers `mozilla-actions/sccache-action` on top of Swatinem's
  target cache (with `CARGO_INCREMENTAL=0` as upstream recommends).
  Per-crate hits survive across test / lint / build jobs that
  previously recompiled the same tokio / futures graph three times.
