# watchwoman

A drop-in replacement for Facebook watchman, written in Rust.

## Why

Watchman is slow to start, bloats to gigabytes of RSS on large trees, and
spews warnings about recrawls, inode limits, and state transitions even
when nothing is wrong. Watchwoman aims to speak the same wire protocol
and CLI surface, without the memory footprint or the noise.

## Status

Pre-alpha. The binary can be installed and aliased as `watchman`, but
not every command is wired up yet. See [CHANGELOG](./CHANGELOG.md) for
what currently works and the parity tracking issue on GitHub for what
doesn't.

## Install

```sh
cargo install --path crates/watchwoman
ln -sf "$(which watchwoman)" "$(dirname "$(which watchwoman)")/watchman"
```

The binary dispatches on `argv[0]`, so either name works.

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
