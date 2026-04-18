# Changelog

All notable changes to this project will be documented in this file.
Format follows [Keep a Changelog](https://keepachangelog.com/).

## [0.3.0] - 2026-04-18

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

## [0.2.2] - 2026-04-18

### Fixed

- Orphan-socket wedge: CLI auto-spawn only triggered when the socket
  file was missing. After a `shutdown-server` or a daemon crash, the
  socket file could linger with nobody listening, and every
  subsequent call errored with "Connection refused" until someone
  manually removed it. Now we attempt to connect first; on
  `ConnectionRefused`, we clean the orphan and respawn.

### Added

- Durable triggers. `trigger` / `trigger-del` now persist to
  `<state-dir>/roots/<root-slug>/triggers.json` on write, and
  `register_root` rehydrates them on daemon start, re-spawning each
  trigger's tick loop. Daemon restarts are invisible to tools that
  installed triggers earlier.
- `watchman-diag` companion binary — dumps version, capabilities,
  sockname, pid, watched roots, per-root cursors / config / triggers
  as one JSON document. Pipe into a gist when opening an issue.
- `watchmanctl` companion binary — thin control wrapper:
  `watchmanctl status | shutdown | log-level [level] | recrawl`.
- CLI subcommands: `debug-recrawl`, `debug-ageout`, `debug-show-cursors`,
  `debug-poll-for-settle` (were accessible only via `-j` before).

## [0.2.1] - 2026-04-18

### Fixed

- Release tarballs now ship every binary the crate builds — the 0.2.0
  tarball only contained `watchwoman` and `watchman`, so `brew
  install` / `mise install` put `watchman-wait` and `watchman-make`
  on your README but not on your `$PATH`.
- Homebrew formula regeneration installs all four binaries (was
  hard-coded to two).

## [0.2.0] - 2026-04-18

Parity push. Every item in `docs/PARITY.md` either ticks now or has
an explicit "intentionally out of scope" note.

### Added

- **Expression terms** — `pcre` and `ipcre` (regex-backed; `regex`
  crate). Capability list now advertises them.
- **Query spec** — `relative_root` narrows the query to a subdir of
  the watched root and strips that prefix from result names;
  `case_sensitive` flag affects `name`/`iname`/`dirname` eval;
  `dedup_results` accepted on spec; `uid`/`gid` fields now populated
  from stat.
- **Named cursors** — `since: "n:my-cursor"` looks up a per-root tick
  cursor and advances it atomically with each query (classic watchman
  semantics).
- **SCM-aware clocks** — `scm:git:<mergebase>` and `scm:hg:<mergebase>`
  (also `scm:sapling:` / `scm:sl:`) shell out to the relevant VCS and
  filter the query to files changed since the merge-base plus any
  uncommitted/untracked changes. Capabilities: `scm-git`, `scm-hg`,
  `scm-since`.
- **Debug commands** — `debug-recrawl` forces a full rescan,
  `debug-ageout` and `debug-poll-for-settle` return clean shapes for
  tools that probe them, `debug-show-cursors` dumps named cursors.
- **Companion binaries** — `watchman-wait` (blocks on matching changes)
  and `watchman-make` (debounced command runner). Both shipped as
  standalone binaries in the release tarballs and via `brew install`.

### Notes

- `sync_timeout`, `lock_timeout`, `settle_period`, `settle_timeout` on
  query specs are accepted but still always settle immediately — our
  event batching runs at 5 ms, so the watchman semantics of "wait
  until the kernel drains" is effectively always true.

## [0.1.2] - 2026-04-18

### Added

- `-j` / `--json-command` — read a JSON PDU from stdin and send it to
  the daemon without going through clap's subcommand surface. Every
  tool in the watchman ecosystem that spawns `watchman -j` (git
  fsmonitor, Sapling, Metro, Jest when BSER isn't available) expects
  this. Pairs with `--no-pretty` for compact output.
- `-p` / `--persistent` — stay connected after the first response and
  stream unilateral PDUs until the daemon closes or SIGINT. Enables
  `watchman -j -p` for long-lived subscriptions.

### Changed

- `watchman` with no args prints help and exits 1 instead of clap's
  default error. Matches upstream behaviour.

## [0.1.1] - 2026-04-18

### Added

- CI/CD pipeline: test + clippy + fmt on every push, gated release on
  Cargo.toml version bump, prebuilt tarballs for six macOS/Linux
  targets (amd64 + arm64, glibc + musl), `cargo publish`, and
  automated Homebrew formula regeneration.

### Changed

- Dropped the `rust-toolchain.toml` pin so CI installs the target for
  the same toolchain it resolves (musl target was missing against
  a 1.94 pin overridden on a stable host).

## [0.1.0] - initial scaffold

### Added

- Cargo workspace scaffolding: `watchwoman`, `watchwoman-protocol`,
  `watchwoman-tests`.
- Async tokio daemon bound to a unix socket. Auto-spawns from the CLI
  when the socket is missing (zeroconf).
- notify-backed watcher with per-root file tree, monotonic clock, and
  coalesced change batches.
- Query engine: `allof`/`anyof`/`not`, `match`/`imatch`/`name`/`iname`,
  `suffix`, `type`, `size`, `exists`, `empty`, `since`, `dirname`;
  generators for `suffix`/`glob`/`path`/`since`; field list covering
  `name`, `size`, `mtime*`, `ctime*`, `type`, `new`, `cclock`, `oclock`,
  `mode`, `ino`, `dev`, `nlink`, `symlink_target`.
- Commands: `get-sockname`, `get-pid`, `version`, `list-capabilities`,
  `watch`, `watch-project`, `watch-list`, `watch-del`, `watch-del-all`,
  `clock`, `query`, `find`, `since`, `subscribe`, `unsubscribe`,
  `flush-subscriptions`, `state-enter`, `state-leave`, `get-config`,
  `log`, `log-level`, `shutdown-server`, plus a `raw` passthrough.
- CLI acts as a client: parses, forwards, and pretty-prints the
  response. `--no-pretty`, `--no-spawn`, and `completion <shell>` flags.
- `watchman` binary alias built from the same source so `$PATH` swaps
  work out of the box.
- Drop-in replacement guide at `docs/REPLACING_WATCHMAN.md`.
- Homebrew tap: `brew install radiosilence/watchwoman/watchwoman`.
- BSER v1 and v2 encoder + decoder with streaming read/write helpers.
  Server sniffs the first byte and speaks whichever encoding the
  client opened with.
- Unilateral subscription push: each subscribe spawns a task that
  watches the root's tick broadcast and streams updates back to the
  connection until it closes.
- Fixture recorder binary (`cargo run -p watchwoman-tests --bin
  record-fixtures`) captures JSON and BSER-v2 responses from the real
  watchman for parity tests.
- `trigger`, `trigger-list`, `trigger-del` commands. Installed triggers
  fork-and-exec on every tick where the expression matches, with
  `append_files` and `stdin: NAME_PER_LINE|json` modes supported.
- `content.sha1hex` field reads file contents on demand and returns the
  lowercase hex-encoded SHA-1 digest.
