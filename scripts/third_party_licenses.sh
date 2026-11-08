#!/usr/bin/env bash
# Writes the license notices of every third-party crate built into the app and the hp3770
# command, which the MIT, Apache, BSD, Zlib and the other licenses ask to ship with the binaries.
# Usage: scripts/third_party_licenses.sh [output]   (default: target/package/THIRD_PARTY_LICENSES.txt)
#
# Needs cargo-about 0.9 (`cargo install --locked --features cli cargo-about`) and the crate
# sources, which any `cargo build --locked` downloads. Configuration: packaging/about.toml and
# the packaging/about.hbs template. scripts/package.sh and scripts/package.ps1 call it.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$root/target/package/THIRD_PARTY_LICENSES.txt}

if ! cargo about --version >/dev/null 2>&1; then
  echo "cargo-about not found: cargo install --locked --features cli cargo-about" >&2
  exit 1
fi

mkdir -p "$(dirname "$out")"
# packaging/about.toml covers every platform, and a build downloads only the crates of its own:
# the others come first, at the versions in Cargo.lock ("failed to download ... --frozen").
(cd "$root" && cargo fetch --locked)
# --frozen: only Cargo.lock and the sources already downloaded, no network. --fail: a crate
# whose license cannot be read stops the release instead of being left out.
(cd "$root" && cargo about generate --frozen --fail --workspace \
  -c packaging/about.toml -o "$out" packaging/about.hbs)
echo "wrote $out"
