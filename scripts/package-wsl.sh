#!/usr/bin/env bash
# Builds the Linux x86_64 release packages in WSL (Ubuntu), without root, for when the
# release workflow cannot run: the same pinned and checksummed packaging tools as the CI
# (keep NFPM/APPIMAGETOOL/APPIMAGE_RUNTIME and cargo-about in step with release.yml), the
# packages checked as the CI does, then copied to $OUT.
#
# It clones $SRC and builds its committed HEAD, not the Windows working copy: commit first.
# From Git Bash on Windows (MSYS_NO_PATHCONV keeps the Linux paths as they are):
#   MSYS_NO_PATHCONV=1 wsl -d Ubuntu -- bash -lc 'SRC=/mnt/c/.../repo OUT=/mnt/c/.../repo/dist/vX.Y.Z bash /mnt/c/.../repo/scripts/package-wsl.sh'
set -euo pipefail
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
SRC=${SRC:?} OUT=${OUT:?}

[ -x "$HOME/.cargo/bin/cargo" ] || curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path >/dev/null
. "$HOME/.cargo/env"

tools=$HOME/hp3770-tools
mkdir -p "$tools"
NFPM_VERSION=2.47.0
APPIMAGETOOL_VERSION=1.9.1
APPIMAGE_RUNTIME_VERSION=20251108
curl -fsSL --retry 5 -o "$tools/nfpm.tar.gz" \
  "https://github.com/goreleaser/nfpm/releases/download/v${NFPM_VERSION}/nfpm_${NFPM_VERSION}_Linux_x86_64.tar.gz"
curl -fsSL --retry 5 -o "$tools/appimagetool" \
  "https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/appimagetool-x86_64.AppImage"
curl -fsSL --retry 5 -o "$tools/appimage-runtime" \
  "https://github.com/AppImage/type2-runtime/releases/download/${APPIMAGE_RUNTIME_VERSION}/runtime-x86_64"
echo "0660ca602b2d2d2ae4781a06c692b3eeb9d437ffea05b831d76e41f4a3188783  $tools/nfpm.tar.gz" | sha256sum -c -
echo "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0  $tools/appimagetool" | sha256sum -c -
echo "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d  $tools/appimage-runtime" | sha256sum -c -
tar -xzf "$tools/nfpm.tar.gz" -C "$tools" nfpm
chmod +x "$tools/nfpm" "$tools/appimagetool"
export NFPM=$tools/nfpm APPIMAGETOOL=$tools/appimagetool APPIMAGE_RUNTIME=$tools/appimage-runtime

# cargo-about 0.9.3, as the CI's install-action: the published Linux binary.
curl -fsSL --retry 5 -o "$tools/about.tar.gz" \
  "https://github.com/EmbarkStudios/cargo-about/releases/download/0.9.3/cargo-about-0.9.3-x86_64-unknown-linux-musl.tar.gz"
tar -xzf "$tools/about.tar.gz" -C "$tools"
install -m755 "$(find "$tools" -name cargo-about -type f | head -1)" "$HOME/.cargo/bin/cargo-about"
cargo about --version

# The commit as pushed, not the Windows working copy.
git config --global --add safe.directory '*'
rm -rf "$HOME/hp3770-work"
git clone -q "$SRC" "$HOME/hp3770-work"
cd "$HOME/hp3770-work"
git log --oneline -1

cargo build --release --locked --bin vetroscan --bin hp3770 --target x86_64-unknown-linux-gnu
scripts/package.sh x86_64-unknown-linux-gnu linux-x86_64 "$(cargo pkgid -p vetroscan | sed 's/.*[#@]//')"

# Smoke test without root: the binary runs, and every package holds what it must.
./target/x86_64-unknown-linux-gnu/release/hp3770 --version
dpkg-deb -c dist/vetroscan-*.deb > /tmp/l
for f in ./usr/bin/vetroscan ./usr/bin/hp3770 ./usr/lib/udev/rules.d/70-hp3770.rules   ./usr/share/applications/vetroscan.desktop ./usr/share/icons/hicolor/scalable/apps/vetroscan.svg   ./usr/share/icons/hicolor/256x256/apps/vetroscan.png ./usr/share/licenses/vetroscan/THIRD_PARTY_LICENSES.txt; do
  grep -q " $f\$" /tmp/l || { echo "deb lacks $f" >&2; exit 1; }
done
test -s dist/vetroscan-*.rpm && test -s dist/vetroscan-*.pkg.tar.zst
appimage=$(realpath dist/vetroscan-*.AppImage)
(cd /tmp && rm -rf squashfs-root && "$appimage" --appimage-extract >/dev/null && test -x squashfs-root/usr/bin/vetroscan   && test -x squashfs-root/usr/bin/hp3770 && test -f squashfs-root/vetroscan.desktop && test -f squashfs-root/vetroscan.png)
echo "SMOKE OK"

mkdir -p "$OUT"
cp dist/* "$OUT"/
ls -la "$OUT"
