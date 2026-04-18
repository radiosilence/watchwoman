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
| **Startup / recrawls**             | Synchronous full crawl on start and on every "recrawl" trigger. Multi-minute on monorepos.           | Inline initial scan using the `ignore` crate; FSEvents recursive mode on macOS; 5 ms event batching.       |
| **`fs.inotify.max_user_watches`**  | Hits the Linux 8192-inode default on modest repos, then burns CPU re-scanning.                       | One registration per root. Irrelevant what the sysctl says.                                                 |
| **Log noise**                      | `RecrawlWarning`, `No watching anymore`, `root dir disappeared` — shouted into every CI log at INFO. | WARN default. Nothing logged unless the kernel actually reported a problem. `RUST_LOG=debug` when needed.  |
| **State corruption**               | Half-written state files wedge the daemon silently; fix is `rm -rf ~/.local/state/watchman`.         | Zero on-disk state. Socket is the only artefact; stale socket is cleaned on start.                         |
| **Crashes**                        | Requires manual restart; subscribers drop silently.                                                   | Next CLI call auto-spawns a fresh daemon. Subscribers reconnect.                                           |
| **Dependencies**                   | C++ + Folly + fbthrift + fizz + wangle + glog + gflags + libsodium + … (≈110 MB of brew deps).       | One ~6 MB static binary. libc.                                                                              |

## Install once, forget forever

Install paths all land the same four binaries on `$PATH`:

```sh
# macOS & Linux, prebuilt
brew install radiosilence/watchwoman/watchwoman
mise use -g "github:radiosilence/watchwoman@latest"

# From source
cargo install watchwoman --bin watchwoman --bin watchman \
                        --bin watchman-wait --bin watchman-make
```

Ships with:

- `watchwoman` — daemon + CLI
- `watchman` — argv-dispatched alias so every existing tool resolves us
- `watchman-wait` — block until a matching file changes
- `watchman-make` — re-run a command on change, debounced

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
- [x] `trigger`, `trigger-list`, `trigger-del`.
- [x] `log`, `log-level`, `shutdown-server`.
- [x] `debug-ageout`, `debug-recrawl`, `debug-show-cursors`, `debug-poll-for-settle`.
- [ ] Trigger persistence across daemon restart — in-memory only.
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
- [ ] `watchman-diag`, `watchmanctl`, `watchman-replicate-subscription` — deferred.

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
