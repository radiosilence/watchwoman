# Watchman parity

This document tracks what watchwoman has, what it's missing, and what
it intentionally refuses.  Ticked items have integration coverage or
a smoke-test against real watchman.

## Wire protocol

- [x] Newline-delimited JSON PDUs (client + server).
- [x] BSER v1 encoder and decoder.
- [x] BSER v2 framing (magic + capability bitmask + length-prefixed payload).
- [x] Template encoding with SKIP tags for absent rows.
- [x] First-byte sniffing on the daemon.
- [ ] BSER capability bits (`DISABLE_UNICODE`, `DISABLE_UNICODE_FOR_ERRORS`) — accepted on the wire, not yet acted on.
- [ ] `watchman-replicate-subscription` — separate binary, deferred.

## CLI

- [x] `argv[0]` dispatch — `watchman` symlink picks up the binary.
- [x] `-j` / `--json-command` — stdin PDU mode.
- [x] `-p` / `--persistent` — stay connected for unilateral updates.
- [x] `--no-pretty` compact JSON output.
- [x] `--sockname` (env: `$WATCHMAN_SOCK`).
- [x] `completion <shell>` generator.
- [x] Auto-spawn daemon on missing socket.
- [ ] `-o` / `--logfile` — daemon is intentionally logless (tracing to stderr).
- [ ] `--pidfile` — zeroconf daemon has no pidfile; socket presence is the liveness signal.
- [ ] `--inetd` — refuse; unix-socket-only is a deliberate choice.

## Commands

- [x] `get-sockname`, `get-pid`, `version`, `list-capabilities`.
- [x] `watch`, `watch-project`, `watch-list`, `watch-del`, `watch-del-all`.
- [x] `clock` — with SCM-aware extension.
- [x] `query`, `find`, `since`.
- [x] `subscribe`, `unsubscribe`, `flush-subscriptions`.
- [x] `state-enter`, `state-leave`.
- [x] `trigger`, `trigger-list`, `trigger-del` — persisted to
      `<state-dir>/roots/<root-slug>/triggers.json`, rehydrated on
      daemon start.
- [x] `get-config`, `log`, `log-level`.
- [x] `shutdown-server`.
- [x] `debug-ageout`, `debug-recrawl`, `debug-show-cursors`.
- [ ] `debug-drop-privs` — refuse; we never run as root by design.
- [ ] `debug-poll-for-settle` — a stub returning immediately.

## Query language

### Expressions

- [x] `allof`, `anyof`, `not`, `true`, `false`.
- [x] `name`, `iname`.
- [x] `match`, `imatch` (glob).
- [x] `pcre`, `ipcre` (regex via `regex` crate).
- [x] `suffix`, `type`, `size`, `exists`, `empty`, `since`, `dirname`, `idirname`.

### Generators

- [x] `glob`, `suffix`, `path`, `since`, `all`.
- [x] `relative_root` — queries rooted at a subdir of the watched tree.

### Options

- [x] `fields` (see below).
- [x] `expression` / `since`.
- [x] `case_sensitive` flag.
- [x] `dedup_results` in subscriptions.
- [x] `empty_on_fresh_instance`, `always_include_directories`, `omit_changed_files`.
- [ ] `sync_timeout`, `lock_timeout`, `settle_period`, `settle_timeout` — accepted, but we always settle immediately.

### Fields

- [x] `name`, `exists`, `type`, `new`.
- [x] `size`, `mode`, `uid`, `gid`, `ino`, `dev`, `nlink`.
- [x] `mtime`, `mtime_ms`, `mtime_ns`, `ctime`, `ctime_ms`, `ctime_ns`.
- [x] `cclock`, `oclock`.
- [x] `symlink_target`.
- [x] `content.sha1hex` — streamed hash on demand.

## Clocks

- [x] `c:<start>:<pid>:<root>:<tick>` — opaque clock string.
- [x] `<integer>` — bare tick number.
- [x] `n:<cursor>` — named cursors, persisted in the root.
- [x] `scm:git:<mergebase>` — git-aware clock; walks `git diff` under the hood.
- [x] `scm:hg:<mergebase>` — Mercurial / Sapling-aware clock.

## Watchers

- [x] FSEvents on macOS (recursive, one registration per root).
- [x] inotify on Linux (coalesced at 5 ms).
- [x] kqueue on the BSDs (via `notify`).
- [ ] Windows `ReadDirectoryChangesW` — not supported; see below.

## Companion binaries

- [x] `watchwoman` (the entrypoint).
- [x] `watchman` (argv-dispatched alias).
- [x] `watchman-wait` — blocks until a matching file changes, prints names.
- [x] `watchman-make` — re-runs a command on matching changes, throttled.
- [x] `watchman-diag` — dumps full daemon state as JSON.
- [x] `watchmanctl` — `status`/`shutdown`/`log-level`/`recrawl`.

## Platforms

- [x] macOS (arm64, amd64).
- [x] Linux (arm64, amd64; glibc and musl).
- [ ] Windows — intentionally out of scope.  The daemon relies on
      unix sockets, `setsid`, and FSEvents/inotify; a proper Windows
      port would need named pipes, a different watcher backend, and
      a different daemonisation path.

## Capability advertisement

Every term and command above is advertised in `list-capabilities`
under the matching `term-*` / `cmd-*` key.  Plus: `bser-v2`,
`wildmatch`, `wildmatch-multislash`, `relative_root`, `dedup_results`,
`clock-sync-timeout`, `scm-git`, `scm-hg`, `scm-since`.
