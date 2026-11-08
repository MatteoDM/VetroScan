#!/usr/bin/env bash
# Packages a release build into dist/ for Linux or macOS (Windows: scripts/package.ps1).
# Usage: scripts/package.sh
#        scripts/package.sh <rust-target|""> <platform-label> <version>
#
#   Without arguments it builds the release binaries for this machine, takes the version from
#   Cargo.toml and the label from the OS and architecture (linux-x86_64, macos-arm64, ...).
#   With arguments (as CI calls it) it packages binaries that are already built.
#
#   Both ship the app (hp3770-app) and the command (hp3770).
#   macOS: hp3770-v<version>-<label>.dmg with "HP Scanjet 3770.app" (ad-hoc signed, not
#          notarized; the hp3770 command is inside it) and a link to /Applications.
#   Linux: hp3770-v<version>-<label>.{deb,rpm,pkg.tar.zst,AppImage}. The deb, rpm and pacman
#          packages also install the udev rule; the AppImage cannot, and the app offers to.
#          Needs nfpm and appimagetool, from PATH or from the NFPM and APPIMAGETOOL variables;
#          APPIMAGE_RUNTIME, if set, is the AppImage runtime file to use.
#   Every package ships THIRD_PARTY_LICENSES.txt, written by scripts/third_party_licenses.sh
#   (needs cargo-about).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)

if [[ $# -eq 0 ]]; then
  target=""
  case "$(uname -s)" in
    Darwin) label="macos-$(uname -m)" ;; # arm64 or x86_64, like the CI labels
    *) label="linux-$(uname -m)" ;;
  esac
  # `cargo pkgid` prints path+file:///.../crates/hp3770-app#0.1.0 (or ...#hp3770-app@0.1.0).
  version=$(cd "$root" && cargo pkgid -p hp3770-app)
  version=${version##*[#@]}
  (cd "$root" && cargo build --release --locked --bin hp3770-app --bin hp3770)
elif [[ $# -eq 3 ]]; then
  target=$1
  label=$2
  version=$3
else
  echo "usage: $0 [<rust-target|\"\"> <platform-label> <version>]" >&2
  exit 1
fi
bindir="$root/target/${target:+$target/}release"
dist="$root/dist"
name="hp3770-v${version}-${label}"

for bin in hp3770-app hp3770; do
  if [[ ! -x "$bindir/$bin" ]]; then
    echo "missing release binary: $bindir/$bin" >&2
    exit 1
  fi
done

mkdir -p "$dist"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

# The notices of the crates built into the binaries, next to LICENSE in every package.
notices="$root/target/package/THIRD_PARTY_LICENSES.txt"
"$root/scripts/third_party_licenses.sh" "$notices"

package_macos() {
  local app="$stage/dmg/HP Scanjet 3770.app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  cp "$bindir/hp3770-app" "$bindir/hp3770" "$app/Contents/MacOS/"
  cp "$root/LICENSE" "$app/Contents/Resources/LICENSE"
  cp "$notices" "$app/Contents/Resources/THIRD_PARTY_LICENSES.txt"
  # icon.icns from the 1024 px icon, with the system's own tools.
  local iconset="$stage/icon.iconset" size
  mkdir -p "$iconset"
  for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$root/packaging/icons/hp3770-app-1024.png" \
      --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) "$root/packaging/icons/hp3770-app-1024.png" \
      --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
  done
  iconutil -c icns "$iconset" -o "$app/Contents/Resources/icon.icns"
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>HP Scanjet 3770</string>
  <key>CFBundleDisplayName</key><string>HP Scanjet 3770</string>
  <key>CFBundleIdentifier</key><string>io.github.matteodm.hp3770</string>
  <key>CFBundleExecutable</key><string>hp3770-app</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDevelopmentRegion</key><string>it</string>
  <key>CFBundleLocalizations</key><array><string>it</string><string>en</string></array>
</dict>
</plist>
PLIST
  # Ad-hoc signature: the app runs on Apple Silicon and is not quarantined as a broken bundle.
  codesign --force --sign - "$app/Contents/MacOS/hp3770"
  codesign --force --sign - "$app"
  ln -s /Applications "$stage/dmg/Applications"
  hdiutil create -volname "HP Scanjet 3770 ${version}" -srcfolder "$stage/dmg" -ov -format UDZO \
    "$dist/$name.dmg" >/dev/null
  echo "packaged $dist/$name.dmg"
}

package_linux() {
  local nfpm=${NFPM:-nfpm} appimagetool=${APPIMAGETOOL:-appimagetool}
  local pkg_arch appimage_arch
  case "$label" in
    *x86_64*) pkg_arch=amd64 appimage_arch=x86_64 ;;
    *aarch64*) pkg_arch=arm64 appimage_arch=aarch64 ;;
    *) echo "unknown Linux architecture in label: $label" >&2; exit 1 ;;
  esac

  # deb, rpm and pacman from one nFPM configuration (paths in it are relative to the root, and
  # it takes the binaries from target/package, whatever target they were built for).
  mkdir -p "$root/target/package"
  cp "$bindir/hp3770-app" "$bindir/hp3770" "$root/target/package/"
  local format extension
  for format in deb rpm archlinux; do
    case "$format" in
      deb) extension=deb ;;
      rpm) extension=rpm ;;
      archlinux) extension=pkg.tar.zst ;;
    esac
    (cd "$root" && HP3770_VERSION="$version" HP3770_ARCH="$pkg_arch" \
      "$nfpm" package --config packaging/linux/nfpm.yaml --packager "$format" \
      --target "$dist/$name.$extension" >/dev/null)
    echo "packaged $dist/$name.$extension"
  done

  # AppImage: a single executable that runs without installing anything. It cannot install the
  # udev rule: the app's "Allow access to the scanner" button does, with one pkexec.
  local appdir="$stage/HP3770.AppDir"
  mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" \
    "$appdir/usr/share/icons/hicolor/scalable/apps" "$appdir/usr/share/licenses/hp3770"
  cp "$bindir/hp3770-app" "$bindir/hp3770" "$appdir/usr/bin/"
  cp "$root/LICENSE" "$notices" "$appdir/usr/share/licenses/hp3770/"
  cp "$root/packaging/linux/hp3770-app.desktop" "$appdir/hp3770-app.desktop"
  cp "$root/packaging/linux/hp3770-app.desktop" "$appdir/usr/share/applications/"
  cp "$root/packaging/icons/hp3770-app-256.png" "$appdir/hp3770-app.png"
  cp "$root/packaging/linux/hp3770-app.svg" "$appdir/usr/share/icons/hicolor/scalable/apps/"
  ln -s usr/bin/hp3770-app "$appdir/AppRun"
  # Extract-and-run: no FUSE needed on build machines and CI runners. APPIMAGE_RUNTIME (CI: the
  # pinned one) instead of the "continuous" runtime that appimagetool would download.
  local runtime=()
  if [[ -n "${APPIMAGE_RUNTIME:-}" ]]; then runtime=(--runtime-file "$APPIMAGE_RUNTIME"); fi
  ARCH="$appimage_arch" "$appimagetool" --appimage-extract-and-run --no-appstream "${runtime[@]}" \
    "$appdir" "$dist/$name.AppImage" >/dev/null
  echo "packaged $dist/$name.AppImage"
}

case "$(uname -s)" in
  Darwin) package_macos ;;
  *) package_linux ;;
esac
