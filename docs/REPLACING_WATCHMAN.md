# Replacing watchman with watchwoman

watchwoman ships a `watchman` binary alongside `watchwoman`. Pointing
`$PATH` at watchwoman is enough to switch every tool that shells out to
`watchman` — Jest, Metro, React Native, Mercurial, fsmonitor, Sapling,
buck2, you name it.

## 1. Stop the old watchman daemon

```sh
watchman shutdown-server 2>/dev/null || true
launchctl list | grep -i watchman
launchctl bootout "$(launchctl list | awk '/watchman/ {print $3; exit}')" 2>/dev/null || true
pkill -f watchman || true
```

Wipe the state dir so clients start clean:

```sh
rm -rf ~/.local/state/watchman
rm -rf "$TMPDIR"/*-state
```

## 2. Install watchwoman

### cargo

```sh
cargo install --git https://github.com/radiosilence/watchwoman \
  --bin watchwoman --bin watchman --root "$HOME/.cargo"
```

### brew

```sh
brew install radiosilence/watchwoman/watchwoman
```

### mise

```sh
mise use -g "cargo:watchwoman@latest"
# or pin:
# mise use -g "cargo:watchwoman@0.1.0"
```

Both binaries (`watchwoman` and `watchman`) land in the same `bin/` dir.

## 3. Uninstall the old watchman

Homebrew:

```sh
brew uninstall --ignore-dependencies watchman
brew untap facebook/fb 2>/dev/null || true
```

MacPorts / apt / yum: remove the package.

The `watchman` from the watchwoman install now shadows any system
version because it's earlier on `$PATH`. Verify:

```sh
which watchman       # → .../bin/watchman (watchwoman)
watchman --version   # → "2026.xx.xx.xx" (watchman compat string)
watchwoman --version # → watchwoman's own semver
```

## 4. Restart your tooling

Kill any long-running clients that already opened a socket to the
old daemon so they rebind:

```sh
pkill -f jest
pkill -f metro
pkill -f 'node.*react-native'
hg --config ui.fncache=0 debugrebuildfncache 2>/dev/null || true
```

Then start them again. They'll auto-spawn watchwoman on first watch.

## 5. Optional: point `$WATCHMAN_SOCK` explicitly

Most tools discover the socket by running `watchman get-sockname`. If a
tool hardcodes a path, set the env var:

```sh
export WATCHMAN_SOCK="$HOME/.local/state/watchman/$USER-state/sock"
```

## Rollback

Install the old watchman with your original package manager; its
binary will precede watchwoman on `$PATH` again. Every watchwoman state
directory is self-contained, so nothing needs cleaning up.

## What's not yet covered

- `trigger`, `trigger-list`, `trigger-del` — return `not implemented`.
- BSER binary protocol — JSON only for now; language bindings that
  hardcode BSER (pywatchman, rust watchman-client in BSER mode) aren't
  usable yet.
- SCM-aware clocks (`scm:hg:...`, `scm:git:...`).

Follow [the parity tracker](https://github.com/radiosilence/watchwoman/issues/1)
for progress.
