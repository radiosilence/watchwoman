# watchwoman

A drop-in, wire-compatible replacement for Facebook watchman. Install
once and everything that speaks watchman — Jest, Metro, Sapling,
Mercurial fsmonitor, git fsmonitor, Buck2, your shell scripts — keeps
working, minus the RAM bloat and log spam.

```sh
brew install radiosilence/watchwoman/watchwoman
```

## Why replace watchman

| Pain                               | watchman                                                                                             | watchwoman                                                                                                 |
|------------------------------------|------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------|
| **Memory**                         | Append-only cache grows unbounded — routinely 1–3 GB after a workday on active repos.                | One `BTreeMap<PathBuf, FileEntry>` per root, bounded by live file count. No historical ballast.            |
| **Startup / recrawls**             | Synchronous full crawl on start and on every "recrawl" trigger. Multi-minute on monorepos.           | Inline initial scan with a manual directory walker; FSEvents recursive mode on macOS; 5 ms event batching.  |
| **`fs.inotify.max_user_watches`**  | Hits the Linux 8192-inode default on modest repos, then burns CPU re-scanning.                       | One registration per root. Irrelevant what the sysctl says.                                                 |
| **Log noise**                      | `RecrawlWarning`, `No watching anymore`, `root dir disappeared` — shouted into every CI log at INFO. | WARN default. Nothing logged unless the kernel actually reported a problem. `RUST_LOG=debug` when needed.  |
| **State corruption**               | Half-written state files wedge the daemon silently; fix is `rm -rf ~/.local/state/watchman`.         | Zero on-disk state (modulo opt-in trigger persistence). Socket is the only artefact, cleaned on start.     |
| **Crashes**                        | Requires manual restart; subscribers drop silently.                                                   | Next CLI call auto-spawns a fresh daemon. Subscribers reconnect.                                           |
| **Abandoned watches**              | Long-lived daemon (LaunchAgent, systemd) never drops a watch; deleted worktrees keep their file tree in RAM until restart. | Background GC every 60 s: reaps roots whose directory has disappeared (≥2 consecutive missed stats) and idle watches (no subs/triggers, untouched 14 d). Zero config. |
| **Visibility**                     | `debug-status` returns a bare roots array; you stitch the rest from `ps`, `watch-list`, per-root `debug-root-status`. | `watchman status` — uptime, RSS, CPU, per-root file counts, idle time, health, and the last 64 reaps in one command. `--json` for scripts. |
| **Dependencies**                   | C++ + Folly + fbthrift + fizz + wangle + glog + gflags + libsodium + … (≈110 MB of brew deps).       | One ~3 MB stripped binary. libc.                                                                           |

## Benchmarks

Measured against real watchman `2026.03.30.00` on macOS arm64, both
daemons on isolated unix sockets, same `.watchmanconfig` on each root
so `node_modules` is excluded identically. Projects are freshly
scaffolded `create-vite`, `create-next-app`, and `create-expo-app`
(with their full `node_modules` installed). Full matrix and script
in [`bench/`](./bench/) once I split it out of the battle-test hack.

| metric                     | watchwoman |   watchman   | speedup          |
|----------------------------|-----------:|-------------:|:----------------:|
| cold scan (vite/next/rn)   |    ~26 ms  |    ~112 ms   | **4.2× faster**  |
| query, 10-iter median      |    3.7 ms  |    100.0 ms  | **27× faster**   |
| scan 10 k-file stress dir  |     48 ms  |     120 ms   | **2.5× faster**  |
| RSS (3 roots watched)      |   9.6 MB   |    26.5 MB   | **2.8× smaller** |
| RSS (after 10 k stress)    |   16 MB    |    29 MB     | **1.8× smaller** |

Query result counts match exactly on every tested project after the
parity fixes in 0.2.3 landed (Vite: 23/23 files, Next: 21/21, Expo:
24/24 — including the documented `ignore_vcs` quirk where `.git` and
its immediate children are reported but not recursed).

watchman hasn't materially changed on these numbers in years — memory
bloats over a session, query time is dominated by a fixed ~100 ms
cookie-based synchronisation overhead. Watchwoman batches events at
5 ms instead and has no cookie round-trip, hence the 27× query win.

## Install once, forget forever

Install paths all land the same six binaries on `$PATH`:

```sh
# mise — recommended; pulls the prebuilt tarball for your OS/arch
mise use -g "github:radiosilence/watchwoman@latest"

# Homebrew — macOS & Linux
brew install radiosilence/watchwoman/watchwoman

# cargo — source build; always latest commit
cargo install watchwoman
```

Ships with:

- `watchwoman` — daemon + CLI
- `watchman` — argv-dispatched alias so every existing tool resolves us
- `watchman-wait` — block until a matching file changes
- `watchman-make` — re-run a command on change, debounced
- `watchman-diag` — dump daemon state as one JSON blob
- `watchmanctl` — `status` / `shutdown` / `log-level` / `recrawl`

The daemon auto-spawns on the first CLI call and stays alive until
the socket disappears. Replacing an existing watchman install:
[`docs/REPLACING_WATCHMAN.md`](./docs/REPLACING_WATCHMAN.md).

Shell completions:

```sh
watchwoman completion zsh  > ~/.zsh/completions/_watchwoman
watchwoman completion bash > /etc/bash_completion.d/watchwoman
watchwoman completion fish > ~/.config/fish/completions/watchwoman.fish
```

## Platforms

Linux and macOS, amd64 and arm64 (glibc and musl on Linux).

Windows is intentionally out of scope — the daemon is built on unix
sockets and `setsid`, and the watcher backend is FSEvents / inotify /
kqueue. A Windows port would need named pipes and
ReadDirectoryChangesW.

## Parity

Ticked items have integration coverage or a smoke-test against real
watchman. Unticked items are either on the list or flagged as
intentionally out of scope with a reason. [Issue
#1](https://github.com/radiosilence/watchwoman/issues/1) tracks the
open slice.

### Wire protocol

- [x] Newline-delimited JSON PDUs.
- [x] BSER v1 + v2 framing, encoder, and decoder.
- [x] Template encoding with SKIP tags.
- [x] First-byte sniffing on the daemon.
- [ ] BSER capability bits (`DISABLE_UNICODE*`) — accepted on the wire, not yet acted on.

### CLI

- [x] `argv[0]` dispatch (`watchman` alias).
- [x] `-j` / `--json-command` — stdin PDU mode.
- [x] `-p` / `--persistent` — stay connected for unilateral updates.
- [x] `--no-pretty`, `--sockname` (env `$WATCHMAN_SOCK`).
- [x] Auto-spawn daemon on missing socket.
- [x] `completion <shell>` generator.
- [ ] `-o` / `--logfile`, `--pidfile` — daemon is logless + socket-liveness-only by design.
- [ ] `--inetd` — unix-socket-only.

### Commands

- [x] `get-sockname`, `get-pid`, `version`, `list-capabilities`, `get-config`.
- [x] `watch`, `watch-project`, `watch-list`, `watch-del`, `watch-del-all`.
- [x] `clock` (SCM-aware), `query`, `find`, `since`.
- [x] `subscribe`, `unsubscribe`, `flush-subscriptions`.
- [x] `state-enter`, `state-leave`.
- [x] `trigger`, `trigger-list`, `trigger-del` — persisted to disk, survive restart.
- [x] `log`, `log-level`, `shutdown-server`.
- [x] `status` — watchwoman-native; human report or `--json`.
- [x] `debug-ageout`, `debug-recrawl`, `debug-show-cursors`, `debug-poll-for-settle`.
- [ ] `debug-drop-privs` — refuse; we never run as root by design.

### Query language

**Expressions.** `allof`, `anyof`, `not`, `true`, `false`, `name`,
`iname`, `match`, `imatch`, `pcre`, `ipcre`, `suffix`, `type`,
`size`, `exists`, `empty`, `since`, `dirname`, `idirname`.

**Generators.** `glob`, `suffix`, `path`, `since`, `all`,
`relative_root`.

**Options.** `fields`, `expression`, `since`, `case_sensitive`,
`dedup_results`, `empty_on_fresh_instance`,
`always_include_directories`, `omit_changed_files`.
`sync_timeout` / `lock_timeout` / `settle_period` / `settle_timeout`
are accepted but we settle in 5 ms anyway.

**Fields.** `name`, `exists`, `type`, `new`, `size`, `mode`, `uid`,
`gid`, `ino`, `dev`, `nlink`, `mtime`, `mtime_ms`, `mtime_ns`,
`ctime`, `ctime_ms`, `ctime_ns`, `cclock`, `oclock`, `symlink_target`,
`content.sha1hex`.

### Clocks

- [x] `c:<start>:<pid>:<root>:<tick>` — opaque clock string.
- [x] `<integer>` — bare tick.
- [x] `n:<cursor>` — named cursors, atomic advance.
- [x] `scm:git:<mergebase>`.
- [x] `scm:hg:<mergebase>` (also Sapling).

### Watchers

- [x] FSEvents on macOS (one registration per root).
- [x] inotify on Linux (5 ms coalescing).
- [x] kqueue on BSD.

### Companion binaries

- [x] `watchwoman`, `watchman`.
- [x] `watchman-wait`, `watchman-make`.
- [x] `watchman-diag`, `watchmanctl`.
- [ ] `watchman-replicate-subscription` — deferred.

## Inspection and GC

`watchman status` prints what the daemon is actually doing — what
upstream watchman makes you stitch together from `ps`, `watch-list`,
and per-root `debug-root-status`:

```
watchwoman 2026.03.30.00  (pid 48769, up 4d03h)
socket:  /Users/you/.local/state/watchman/you-state/sock
memory:  954 MB rss   cpu: 12m34s user / 4m02s system
         421 MB tracked data (est) · 533 MB unaccounted (allocator / OS-held)
roots:   12 watched · 2,804,061 files (2,798,512 live, 5,549 tombstones) · 3 subs · 0 triggers

ROOT                                                    FILES   GHOSTS     MEM~  SUB  TRG  HEALTH
…/workspace/project-a/.worktrees/feature-x            551,889    1,203   88 MB    1    0  active
…/workspace/project-a/.claude/worktrees/agent-a115    223,537        4   34 MB    0    0  idle
…/workspace/project-a/.worktrees/prefetch-search            0        0      0 B    0    0  dead
```

The `tracked data (est)` number is watchwoman's own accounting of the
per-entry struct + path bytes it's holding; `unaccounted` is the rest
of RSS — allocator fragmentation, OS-held pages, BTreeMap node slop.
A big gap there means you're looking at glibc / libmalloc, not at a
leak.  Pass `--json` for scripting; the server always speaks JSON
over the wire and the CLI just formats it.

**Garbage collection is zero-conf.**  Every 60 s the daemon sweeps
every watched root:

- If `stat()` returns ENOENT on **two consecutive ticks** (60–120 s
  grace), the watch is reaped as `dead`.  Usual cause: a removed git
  worktree, an unmounted volume, or a deleted scratch tree.
- If a root has **no subscriptions, no triggers, and no commands have
  touched it for 14 days**, it's reaped as `stale`.  Anything actively
  subscribed or triggered is never stale-reaped, regardless of idle
  time.
- Tombstones (`exists: false` entries kept so `since` queries can
  report deletions) are pruned down to the oldest named cursor's
  watermark.  With no cursors, the prune is immediate — branch
  churn on a monorepo doesn't accumulate ghost entries.  Subscribers
  see deletions live via the tick broadcast and don't block the
  prune.

Reaps log at `WARN` and appear in `watchman status` for the next 64
events so a long-running LaunchAgent can't silently drop a watch you
cared about.

## Development

```sh
cargo build
cargo test                                         # tests against watchwoman
WATCHWOMAN_UNDER_TEST=watchman cargo test          # same tests, real watchman
cargo run -p watchwoman-tests --bin record-fixtures   # refresh parity fixtures
```

Pre-push hooks: `cargo fmt --all` + `cargo clippy --workspace -- -D warnings`.

### Layout

- `crates/watchwoman` — daemon, CLI, query engine, watcher, companion bins.
- `crates/watchwoman-protocol` — JSON and BSER v1/v2 codecs.
- `crates/watchwoman-tests` — black-box harness; every integration
  test also runs against real watchman when `WATCHWOMAN_UNDER_TEST=watchman`.
- `docs/PROTOCOL.md` — wire format reference distilled from upstream.
- `docs/REPLACING_WATCHMAN.md` — step-by-step swap guide.
