# watchwoman

Watchman, but she doesn't eat your RAM and doesn't cry in your logs.

Drop-in wire-compatible replacement. `brew install
radiosilence/watchwoman/watchwoman`, everything that expects
`watchman` keeps working, nothing on your laptop notices she's not
the same bitch from Menlo Park.

## Why

You already know. You've watched `watchman` crawl a monorepo for 89
seconds while Jest waits. You've found a 3 GB RSS on a process you
started six hours ago. You've `rm -rf`'d `~/.local/state/watchman/`
more times than you've pruned docker. Your SRE has muted "recrawl
warning" in Slack. That is not a tool you keep out of affection. It's
Stockholm syndrome with a C++ build.

### Memory

Watchman's cache tree is append-only. It grows with every file
creation and every rename, shrinks approximately never, and in
production on a churny repo routinely gets to several gigs before the
oom-killer or the user intervenes. The official workaround on Facebook's
wiki is "restart it on a cron." That is, in fact, the state of the art.

Watchwoman keeps one `BTreeMap<PathBuf, FileEntry>` per root, drops
entries with absent inodes on the next rescan, and prunes `.git`,
`node_modules`, `target`, `.hg`, `.svn` by default. Your RSS is
bounded by the number of files that *currently exist*, not by the
history of every filename you've ever typed.

### Recrawls

Upstream's recrawl code fires on: dropped inotify events, the
`fs.inotify.max_user_watches` sysctl, FSEvents coalescing quirks, an
SCM state change, and "it felt like it." Each one synchronously
re-scans the whole tree and emits a `RecrawlWarning: ...` line that
your CI log collector dutifully ships to every slack channel that
ever looked at it sideways.

Watchwoman registers one FSEvents stream per root on macOS (recursive,
no `max_user_watches` limit at all), batches inotify events at 5 ms
on Linux, treats dropped events as a hint to reconcile rather than a
panic, and — by default — shuts up.

### Log noise

Default `watchman --foreground` output, taken at random from a Jest
run on a clean laptop:

```
RecrawlWarning: ...
No watching anymore
Watchman was terminated due to a timeout
State enter X was not matched with a state-leave
root dir /Users/... disappeared
```

None of those are errors. Three of them are lies (the root dir did
not disappear). Watchwoman logs at WARN. `RUST_LOG=debug` exists when
you actually want noise.

### State corruption

If two watchmen on different `$PATH`s race for the same socket, the
daemon half-writes its state file, and every subsequent client reads
a truncated JSON. Subscriptions then silently stop delivering. The
documented fix is to nuke the state dir and restart every tool that
held the old socket.

Watchwoman has zero on-disk state. The socket is the only artefact;
it's cleaned on start. If the daemon crashes, the next CLI call
auto-spawns a fresh one. Nothing to corrupt, nothing to nuke.

### Install weight

Upstream: C++17, CMake, Folly, fbthrift, edencommon, fb303, wangle,
fizz, glog, gflags, double-conversion, libsodium. On this laptop
`brew uninstall watchman` freed 110 MB of runtime dependencies you
didn't know you had.

Watchwoman: one Cargo workspace, `cargo build --release` in under a
minute, ~6 MB binary, no dynamic deps beyond libc. Patching it is a
git clone away.

### Platform quirks, briefly

- **Linux**: `fs.inotify.max_user_watches` defaults to 8192. Real
  repos have more files than that. Upstream burns CPU on recrawls
  once you hit the ceiling. We don't, because we use one registration
  per root.
- **macOS**: FSEvents drops events under load. Upstream panics and
  recrawls. We shrug and reconcile on the next query.

## What's the same

Wire protocol (JSON and BSER v1/v2), CLI surface, capability
advertisement, expression language, clock semantics. Every tool
that talks watchman — Jest, Metro, React Native, Sapling, Mercurial
fsmonitor, git fsmonitor, buck2, your hand-rolled `watchman -j`
shell script — talks watchwoman without knowing.

Companions ship too: `watchman-wait`, `watchman-make`, both wired to
the same daemon.

## Install

### Homebrew (recommended)

```sh
brew install radiosilence/watchwoman/watchwoman
```

Installs `watchwoman` and a `watchman` alias side by side, so
`$PATH` swaps Just Work.

### mise

```sh
mise use -g "github:radiosilence/watchwoman@latest"
```

Pulls the prebuilt tarball for your OS/arch from the GitHub release.
No toolchain required.

### cargo (source install, last resort)

```sh
cargo install watchwoman --bin watchwoman --bin watchman
```

Slower, but always the latest commit.

### Replace an existing watchman

See [`docs/REPLACING_WATCHMAN.md`](./docs/REPLACING_WATCHMAN.md) —
kill the old daemon, purge state, install, restart jest/metro/sapling.

### Shell completions

```sh
watchwoman completion zsh  > ~/.zsh/completions/_watchwoman
watchwoman completion bash > /etc/bash_completion.d/watchwoman
watchwoman completion fish > ~/.config/fish/completions/watchwoman.fish
```

## Platforms

Linux and macOS, amd64 and arm64, glibc and musl on Linux.

**Not Windows.** The daemon is unix-socket native and expects
FSEvents/inotify/kqueue. A Windows port would mean named pipes and
ReadDirectoryChangesW — possible, not scheduled. If you need
watchman on Windows, use watchman on Windows.

## Parity

Ticked items have integration coverage or a smoke-test against real
watchman. Unticked items are either on the list or explicitly out of
scope — flagged as such with a reason. [Issue #1](https://github.com/radiosilence/watchwoman/issues/1)
tracks the open slice.

### Wire protocol

- [x] Newline-delimited JSON PDUs (client + server).
- [x] BSER v1 encoder and decoder.
- [x] BSER v2 framing (magic + capability bitmask + length-prefixed payload).
- [x] Template encoding with SKIP tags for absent rows.
- [x] First-byte sniffing on the daemon.
- [ ] BSER capability bits (`DISABLE_UNICODE`, `DISABLE_UNICODE_FOR_ERRORS`) — accepted on the wire, not yet acted on.
- [ ] `watchman-replicate-subscription` — separate binary, deferred.

### CLI

- [x] `argv[0]` dispatch — `watchman` alias picks up the binary.
- [x] `-j` / `--json-command` — stdin PDU mode.
- [x] `-p` / `--persistent` — stay connected for unilateral updates.
- [x] `--no-pretty` compact JSON output.
- [x] `--sockname` (env: `$WATCHMAN_SOCK`).
- [x] `completion <shell>` generator.
- [x] Auto-spawn daemon on missing socket.
- [ ] `-o` / `--logfile` — daemon traces to stderr, no logfile ceremony.
- [ ] `--pidfile` — socket presence is the liveness signal.
- [ ] `--inetd` — unix-socket-only is a deliberate choice.

### Commands

- [x] `get-sockname`, `get-pid`, `version`, `list-capabilities`.
- [x] `watch`, `watch-project`, `watch-list`, `watch-del`, `watch-del-all`.
- [x] `clock` — with SCM-aware extension.
- [x] `query`, `find`, `since`.
- [x] `subscribe`, `unsubscribe`, `flush-subscriptions`.
- [x] `state-enter`, `state-leave`.
- [x] `trigger`, `trigger-list`, `trigger-del`.
- [ ] Trigger persistence across daemon restart — in-memory only for now.
- [x] `get-config`, `log`, `log-level`.
- [x] `shutdown-server`.
- [x] `debug-ageout`, `debug-recrawl`, `debug-show-cursors`, `debug-poll-for-settle`.
- [ ] `debug-drop-privs` — refuse; we never run as root by design.

### Query language

**Expressions**

- [x] `allof`, `anyof`, `not`, `true`, `false`.
- [x] `name`, `iname`.
- [x] `match`, `imatch` (glob).
- [x] `pcre`, `ipcre` (regex).
- [x] `suffix`, `type`, `size`, `exists`, `empty`, `since`, `dirname`, `idirname`.

**Generators**

- [x] `glob`, `suffix`, `path`, `since`, `all`.
- [x] `relative_root` — queries rooted at a subdir of the watched tree.

**Spec options**

- [x] `fields`, `expression`, `since`.
- [x] `case_sensitive`.
- [x] `dedup_results` in subscriptions.
- [x] `empty_on_fresh_instance`, `always_include_directories`, `omit_changed_files`.
- [ ] `sync_timeout`, `lock_timeout`, `settle_period`, `settle_timeout` — accepted, but we settle in 5 ms anyway.

**Fields**

- [x] `name`, `exists`, `type`, `new`.
- [x] `size`, `mode`, `uid`, `gid`, `ino`, `dev`, `nlink`.
- [x] `mtime`, `mtime_ms`, `mtime_ns`, `ctime`, `ctime_ms`, `ctime_ns`.
- [x] `cclock`, `oclock`, `symlink_target`.
- [x] `content.sha1hex` — streamed hash on demand.

### Clocks

- [x] `c:<start>:<pid>:<root>:<tick>` — opaque clock string.
- [x] `<integer>` — bare tick number.
- [x] `n:<cursor>` — named cursors, advance atomically with each query.
- [x] `scm:git:<mergebase>` — git-aware clock.
- [x] `scm:hg:<mergebase>` — Mercurial / Sapling-aware clock.

### Watchers

- [x] FSEvents on macOS (recursive, one registration per root).
- [x] inotify on Linux (coalesced at 5 ms).
- [x] kqueue on the BSDs (via `notify`).
- [ ] Windows `ReadDirectoryChangesW` — out of scope.

### Companion binaries

- [x] `watchwoman` (the entrypoint).
- [x] `watchman` (argv-dispatched alias).
- [x] `watchman-wait` — blocks until a matching file changes, prints names.
- [x] `watchman-make` — re-runs a command on matching changes, throttled.
- [ ] `watchman-diag` — deferred; `watchman --version --capabilities` covers the ask.
- [ ] `watchmanctl` — deferred.

## Development

```sh
cargo build
cargo test                                     # against watchwoman
WATCHWOMAN_UNDER_TEST=watchman cargo test      # against real watchman
cargo run -p watchwoman-tests --bin record-fixtures   # refresh parity fixtures
```

Pre-push hooks: `cargo fmt --all` + `cargo clippy --workspace -- -D warnings`.

## Layout

- `crates/watchwoman` — daemon, CLI, query engine, watcher, companion bins.
- `crates/watchwoman-protocol` — JSON and BSER v1/v2 codecs.
- `crates/watchwoman-tests` — black-box harness that spawns either
  `watchwoman` or real `watchman` via `WATCHWOMAN_UNDER_TEST`. Every
  integration test is a parity test.
- `docs/PROTOCOL.md` — wire format reference distilled from upstream.
- `docs/PARITY.md` — feature matrix.
- `docs/REPLACING_WATCHMAN.md` — step-by-step swap guide.
- `reference/watchman` — upstream source, gitignored.
