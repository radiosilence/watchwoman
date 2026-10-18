### Added

- CI/CD pipeline: test + clippy + fmt on every push, gated release on
  Cargo.toml version bump, prebuilt tarballs for six macOS/Linux
  targets (amd64 + arm64, glibc + musl), `cargo publish`, and
  automated Homebrew formula regeneration.

### Changed

- Dropped the `rust-toolchain.toml` pin so CI installs the target for
  the same toolchain it resolves (musl target was missing against
  a 1.94 pin overridden on a stable host).
