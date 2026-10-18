Battle-test release: parity against real watchman 2026.03.30.00
verified on Vite / Next / Expo projects + a 10k-file stress tree.
File counts match exactly, all volatile-masked command outputs
match, and watchwoman is faster on every dimension measured.

### Fixed

- Every response now includes `"version"` at the top level. Upstream
  always emits it; jest/pywatchman warn or outright refuse without.
- `fields:["name"]` returns a flat array of strings — upstream's
  documented shortcut — instead of an array of `{"name": ...}`.
- `.watchman-cookie-*` files (watchman's sync cookies) filtered out
  of query results.
- `always_include_directories` defaults to `true`, matching real
  watchman's actual behaviour regardless of what its docs say.
- `.watchmanconfig` parsed on `register_root`; `ignore_dirs` is
  respected at every depth, and VCS dirs (`.git`/`.hg`/`.svn`) are
  reported along with their immediate children but not recursed
  deeper — matching upstream's `ignore_vcs` quirk.
- `get-config` returns the actual `.watchmanconfig` contents when the
  caller supplies a watched root that has one.
- Orphan socket recovery (from 0.2.2) confirmed working under
  SIGKILL chaos: CLI auto-cleans and respawns.

### Added

- `watchman-diag` and `watchmanctl` companion binaries.
- Debug stubs so upstream-compatible clients don't crash on
  `debug-status` / `debug-watcher-info` / `debug-get-asserted-states` /
  `debug-get-subscriptions` / `debug-root-status` / `debug-contenthash`.
- `get-log` and `global-log-level` for shape-compatible parity.

### Changed

- Replaced `ignore::WalkBuilder` with a manual directory walker that
  prunes at the dir level.  WalkBuilder descended into `node_modules`
  before the filter applied; the new walker skips the subtree
  entirely, taking cold scans from ~110 ms to ~26 ms on real JS
  projects (4.2× faster) and stress scans from 124 ms to 48 ms (2.5×).
- Help / `--version` cleaned up.  No misleading "symlink `watchman`
  next to the binary" copy — we already ship both bins.

### Benchmarks

Against watchman 2026.03.30.00, macOS arm64, three scaffolded
projects (Vite / Next / Expo) and a 10 k-file stress directory:

| metric                     | watchwoman | watchman  | ratio   |
|----------------------------|-----------:|----------:|:-------:|
| cold scan (median)         |    ~26 ms  |   ~112 ms | 4.2×    |
| query 10-iter median       |    3.7 ms  |   100 ms  | 27×     |
| 10 k-file scan             |     48 ms  |   120 ms  | 2.5×    |
| RSS after 3 roots          |   9.6 MB   |  26.5 MB  | 2.8×    |
| RSS after 10 k stress      |  16.1 MB   |  29.1 MB  | 1.8×    |
