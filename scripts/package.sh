#!/usr/bin/env bash
# Packages a release build of every watchwoman binary for one target into
# dist/watchwoman-<version>-<target>.tar.gz and its .sha256.
set -euo pipefail

TARGET="$1"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME="watchwoman-${VERSION}-${TARGET}"

mkdir -p "dist/${NAME}"
for bin in watchwoman watchman watchman-wait watchman-make watchman-diag watchmanctl; do
  cp "target/${TARGET}/release/${bin}" "dist/${NAME}/"
done
cp README.md CHANGELOG.md LICENSE* "dist/${NAME}/" 2>/dev/null || true
tar -czf "dist/${NAME}.tar.gz" -C dist "${NAME}"
(cd dist && shasum -a 256 "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256")
