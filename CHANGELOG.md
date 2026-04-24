# Changelog

All notable changes to this project will be documented in this file.
Format follows [Keep a Changelog](https://keepachangelog.com/).

## Unreleased

### Added

- Field support for `mtime_us`, `mtime_f`, `ctime_us`, `ctime_f` —
  microsecond and fractional-second variants of the time fields we
  already carry at nanosecond precision.  Pure derivations; no
  memory growth.

## [0.5.1] - 2026-04-24

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

## [0.5.0] - 2026-04-22

### Added

- `watchman status` — human-readable daemon report: uptime, RSS, CPU,
  per-root file counts, idle time, subscription/trigger counts, health
  (`active` / `idle` / `stale` / `dead`), and the last 64 garbage-
  collected watches.  `--json` for scripting; the server response
  always is JSON and the CLI only formats it.  Added
  `cmd-status` to `list-capabilities`.
- Watch garbage collector.  A background sweep, zero-conf, runs every
  60 s and reaps:
    - **dead** roots — directory missing from disk on two consecutive
      ticks (~60-120 s grace); usually a git worktree that was
      removed or a volume that unmounted.
    - **stale** roots — no subscriptions, no installed triggers, and no
      commands for 14 days.  Active roots are never stale-reaped
      regardless of idle time.

  Reaps log at `WARN` so the long-running LaunchAgent can't silently
  throw away a watch you cared about, and they surface in
  `watchman status` for post-mortem.

## [0.4.1] - 2026-04-18

### Fixed

- `install-agent` used `current_exe()` verbatim, which baked
  version-pinned paths like
  `~/.local/share/mise/installs/github-.../0.4.0/watchman` into the
  plist.  Next `mise upgrade` silently orphaned the agent.  The
  resolver now prefers (in order): the mise shim path, the brew
  prefix, `~/.cargo/bin`, and only falls back to `current_exe` with
  a warning if all stable locations are missing.  Re-run
  `watchwoman install-agent` to regenerate a plist with the new path.

## [0.4.0] - 2026-04-18

Parity polish: every CLI flag and command in upstream watchman's
help output is now accepted by watchwoman.  47 commands and ~15
flags audited against `watchman --help` from 2026.03.30.00.

### Added

- CLI flag aliases so tools that hard-code the upstream spelling work
  without adjustment: `-U`/`-u`/`--unix-listener-path` alias for
  `--sockname`, `--foreground` alias for the internal
  `--foreground-daemon`.
- `-o`/`--logfile PATH` — redirects tracing output to the given file
  (append mode); `-` restores stderr.
- `--log-level 0|1|2|3` — maps to `off`/`warn`/`debug`/`trace`.
- `--pidfile PATH` — written on foreground-daemon start, for init
  scripts and process monitors that grep it.
- CLI subcommands added to match `watchman --help`: `find`, `since`,
  `trigger`, `trigger-list`, `trigger-del`, `get-log`,
  `global-log-level`, `debug-status`, `debug-root-status`,
  `debug-watcher-info`, `debug-watcher-info-clear`,
  `debug-get-asserted-states`, `debug-get-subscriptions`,
  `debug-contenthash`, `debug-kqueue-and-fsevents-recrawl`,
  `debug-fsevents-inject-drop`, `debug-set-parallel-crawl`,
  `debug-set-subscriptions-paused`, `debug-symlink-target-cache`.

### Removed

- Accept-and-ignore-only CLI compat flags that no real tool in the
  ecosystem actually passes: `--inetd`, `-S`/`--no-site-spawner`,
  `-n`/`--no-save-state`, `--statefile`, `--no-local`,
  `--named-pipe-path`, `--pretty`.  Parity is carried by the flags
  that do something; the rest were cargo-cult.

### Verified

End-to-end tested 49 subcommand + flag scenarios against a live
daemon; each returns a shape-compatible JSON object.  Parity with
watchman 2026.03.30.00 for everything a real tool uses.

## [0.3.1] - 2026-04-18

### Added

- `watchwoman install-agent` / `uninstall-agent` on macOS — drops a
  LaunchAgent plist at `~/Library/LaunchAgents/cc.blit.watchwoman.plist`
  and bootstraps it with `launchctl`, so the daemon auto-starts at
  login and restarts on crash.  Not run automatically; opt in.

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
