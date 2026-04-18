# Changelog

All notable changes to this project will be documented in this file.
Format follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

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
