### Fixed

- Release tarballs now ship every binary the crate builds — the 0.2.0
  tarball only contained `watchwoman` and `watchman`, so `brew
  install` / `mise install` put `watchman-wait` and `watchman-make`
  on your README but not on your `$PATH`.
- Homebrew formula regeneration installs all four binaries (was
  hard-coded to two).
