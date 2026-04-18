# watchwoman

A drop-in replacement for Facebook watchman, written in Rust.

## Why

If you've ever stared at a 3 GB `watchman` process on your laptop while
Jest prints "Watchman crawl took 89412ms, which is longer than expected",
you already know.

### Pain: memory

Watchman keeps every discovered path in an append-only cache tree. The
structure grows as files come and go and never shrinks — the "ageout"
sweeper is conservative by design, and on a repo with churny tests or a
chatty build step you can watch RSS climb from 80 MB to 2 GB over a
workday. Workarounds in the wild include cron jobs that restart
`watchman` every few hours.

Watchwoman keeps a single BTreeMap per root keyed by relative path,
recycles deleted entries during `apply_changes`, and skips `.git`,
`node_modules`, `target`, `.hg`, `.svn` by default. Same workload,
roughly one pointer-plus-stat per live file, no historical ballast.

### Pain: startup and recrawls

Watchman does a synchronous full crawl whenever it establishes a watch,
restarts, or "recrawls" — and recrawls fire for reasons like the kernel
dropping an event, hitting the Linux `fs.inotify.max_user_watches`
ceiling, or an FSEvents coalescing glitch. On a monorepo that's three
minutes before Jest can even start. Every recrawl also emits a log line
that CI's log collectors dutifully ship to your SRE team.

Watchwoman's initial scan runs inline during `watch-project`, uses the
`ignore` crate's walker, and on macOS leans on FSEvents's recursive mode
(one registration for the whole root, no `inotify.max_user_watches` to
run into). Re-scans are incremental and do not emit noise unless the
kernel actually signalled a problem.

### Pain: noise

`watchman --foreground` routinely logs at INFO:

- `RecrawlWarning: ...`
- `No watching anymore`
- `Watchman was terminated due to a timeout`
- `State enter ... was not matched with a state-leave`
- periodic `root dir X disappeared` for directories that very much exist

These are shouted into every CI log whether or not they affect the
caller. Watchwoman logs at WARN by default and at DEBUG for anything
non-fatal; `RUST_LOG=debug` turns the firehose on when you actually
want it.

### Pain: state corruption

If watchman's state dir gets wedged — usually because a previous
process crashed mid-write, or two watchmen on different PATHs raced
for the same socket — the fix is `rm -rf ~/.local/state/watchman` and
restart every tool that held the old socket. There is no sharp error
message; subscriptions just silently stop delivering.

Watchwoman keeps zero on-disk state. The socket path is the only
artefact, it is cleaned up on start, and the daemon is
single-source-of-truth for roots and clocks. If it crashes, the next
CLI call auto-spawns a fresh one.

### Pain: build and install

Upstream watchman is a C++ + CMake + Folly + gflags + glog + libevent
affair that takes real effort to build from source. Patching it
requires owning a facebook/fb build environment. The Homebrew bottle
is the realistic path for most people, and it pulls in a half-dozen
facebook libraries as runtime deps.

Watchwoman is one Cargo workspace. `cargo build --release` takes under
a minute on a laptop, binary comes in around 6 MB, and the only
runtime dependency is libSystem / libc.

### Pain: platform quirks

- **Linux**: `fs.inotify.max_user_watches` is often 8192 by default;
  watchman hits that ceiling on any modest repo and then starts
  burning CPU on recrawls. Watchwoman batches at 5 ms, which means
  one watch registration per root even on inotify.
- **macOS**: FSEvents occasionally drops events under load and
  watchman's response is to recrawl. Watchwoman treats dropped events
  as hints rather than alarms and reconciles the tree on the next
  query.

### What's the same

Wire protocol, CLI surface, capability advertisement, and the
expression language. If your tool already talks to watchman, it does
not need to know watchwoman exists.

### Platforms

Linux (glibc and musl) and macOS, amd64 and arm64. Windows is not
supported — the daemon leans on unix sockets and unix-only syscalls;
a Windows port would need named pipes and a different watcher
backend, tracked as a separate piece of work.

### What's not yet ported

SCM-aware clocks (`scm:hg:...`, `scm:git:...`) and durable triggers
across daemon restart. Tracked on
[issue #1](https://github.com/radiosilence/watchwoman/issues/1).

## Status

Pre-alpha. The binary can be installed and aliased as `watchman`, but
not every command is wired up yet. See [CHANGELOG](./CHANGELOG.md) for
what currently works and the parity tracking issue on GitHub for what
doesn't.

## Install

### Homebrew

```sh
brew install radiosilence/watchwoman/watchwoman
```

Installs both `watchwoman` and `watchman` (drop-in alias).

### mise

```sh
mise use -g "github:radiosilence/watchwoman@latest"
```

Fetches the prebuilt tarball off the GitHub release for your
OS/arch — no local toolchain required.

### cargo (source install)

```sh
cargo install watchwoman --bin watchwoman --bin watchman
```

Builds from source; slower than brew/mise but picks up the latest
commit when paired with `--git`.

### Shell completions

```sh
watchwoman completion zsh  > ~/.zsh/completions/_watchwoman
watchwoman completion bash > /etc/bash_completion.d/watchwoman
watchwoman completion fish > ~/.config/fish/completions/watchwoman.fish
```

### Drop-in replacement

Swapping an existing watchman install: see
[`docs/REPLACING_WATCHMAN.md`](./docs/REPLACING_WATCHMAN.md).

## Development

```sh
cargo build
cargo test                 # runs parity tests against watchwoman
WATCHWOMAN_UNDER_TEST=watchman cargo test   # same tests against real watchman
```

Pre-push hooks run `cargo fmt --all` and `cargo clippy --workspace -- -D warnings`.

## Layout

- `crates/watchwoman` — daemon, CLI, query engine, watcher backend.
- `crates/watchwoman-protocol` — JSON and BSER v1/v2 codecs, shared `Value` type.
- `crates/watchwoman-tests` — black-box integration harness that runs
  against either `watchwoman` or the real `watchman` binary.
- `reference/watchman` — upstream source (gitignored, cloned locally).
