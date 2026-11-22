---
name: release
description: Publishes a version of VetroScan — raises the workspace version, moves what is new in the CHANGELOG, checks and commits; then the vX.Y.Z tag lets the "CI and Release" workflow build and publish the packages (only if GitHub Actions cannot run, the Windows and Linux packages are made locally and published with gh release create); also installs the new version on this PC and recovers from a failed release. Use it when asked to release, publish, tag, raise the version, build the packages or install the new version.
disable-model-invocation: true
model: sonnet
---

# Releasing the scanner

The packages are made by the `CI and Release` workflow (`.github/workflows/release.yml`). A
release is just a version bump plus a matching tag.

Pipeline: `test` (Linux, Windows, macOS) and `lint` (Linux: `cargo fmt --all --check`, clippy
with `-D warnings`) → `prepare` (reads the version of the `vetroscan` package with
`cargo metadata`; stops if the tag is not exactly `v<version>`) → `build` (5 targets: linux
x86_64/aarch64, windows x86_64, macOS arm64/x86_64; each packages and then tries what it
produced) → `release` (only on `v*` tags: creates the GitHub release with the CHANGELOG's
notes, uploads the files and `SHA256SUMS.txt`).

Files of every release (each with `vetroscan` and `hp3770`): Windows
`…-windows-x86_64-setup.exe` (Inno Setup, `packaging/windows/vetroscan.iss`; by default a
per-user install without administrator) and `…-windows-x86_64-portable.zip`; macOS
`…-macos-{arm64,x86_64}.dmg`; Linux `…-linux-{x86_64,aarch64}.{deb,rpm,pkg.tar.zst,AppImage}`
(nFPM `packaging/linux/nfpm.yaml` + appimagetool). deb, rpm and pacman install the udev rule
in `/usr/lib/udev/rules.d` and apply it with `packaging/linux/postinstall.sh`; the AppImage
cannot, and the app offers the button for the permission. Versions and SHA256 of nfpm,
appimagetool and the AppImage runtime are pinned in the *Install Linux packaging tools*
step: they are updated together (the SHA256 of GitHub assets are read from the `digest`
field of `gh api repos/<owner>/<repo>/releases/tags/<tag>`). Every package also has
`THIRD_PARTY_LICENSES.txt`, written by cargo-about (`taiki-e/install-action`, version pinned
in the build job) from `packaging/about.toml` and `packaging/about.hbs`; the package tests
check it is there. A new dependency with a licence missing from `packaging/about.toml`
blocks the build: add it there and in `deny.toml` (workflow *Dependencies*,
`cargo deny check`), only if it is compatible with GPL-2.0-or-later. Never change the `AppId`
in `vetroscan.iss`: Windows uses it to update and uninstall. A tag with `-` (e.g.
`v0.2.0-rc.1`) comes out as a prerelease. `workflow_dispatch` builds the packages without
publishing them.

The AUR PKGBUILD `packaging/aur/PKGBUILD` (`vetroscan-git`) builds from git and does not follow
the tags: it is not touched at every release.

## Steps

1. Start from a clean, up-to-date `main` (`git status`, `git pull --ff-only`). Look at the
   last tag with `git tag --sort=-v:refname | head -1` and ask the user which version to
   release if they did not say (patch/minor/major according to semver).
2. Raise `version` under `[workspace.package]` in the root `Cargo.toml`: every crate inherits
   it (`hp3770 --version` reads it from there). In `CHANGELOG.md` rename `## [Unreleased]` to
   `## [X.Y.Z] - YYYY-MM-DD`, add an empty `## [Unreleased]` above it and update the links at
   the bottom (`[Unreleased]` compares `vX.Y.Z...HEAD`, `[X.Y.Z]` compares the previous tag
   with `vX.Y.Z`). Check it against `git log --oneline v<previous>..HEAD`: every change
   visible to the user goes under *Added*, *Changed*, *Removed* or *Fixed*, written for
   whoever uses the scanner; fixes of things never released are not listed. The `release`
   job publishes that section as notes and fails if it is missing.
3. Update the lockfile and check:
   ```bash
   cargo build --workspace
   cargo test --workspace --locked
   cargo fmt --all --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   ```
   These are Windows' only: the push to `main` runs the tests on Linux, Windows and macOS.
   Do not build the Linux or macOS packages, nor run the WSL checks, here: the pipeline does
   (Matteo, 22 November 2026).
4. Commit: `Release vX.Y.Z` with `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md`.
   The workflow runs (`gh run list --limit 3`: the repository is public again since
   22 November 2026; from 10 to 22 November it was private and Actions did not start, the
   reason for *While the workflow cannot run* below). Pushing and tagging are Matteo's: give
   him the commands.
5. Push `main` and **do the dress rehearsal**: *Actions → CI and Release → Run workflow* on
   `main` (or `gh workflow run release.yml --ref main`). It runs the tests, `prepare` and the
   five `build`s but not `release`. Only an all-green rehearsal guarantees that the tag will
   publish: the build stage runs only on tags or by hand, so a green CI on a push says
   nothing about the packages.
6. **Ask the user before creating the tag**: pushing a tag publishes a release. Then:
   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```
7. Follow the run with `gh run list --workflow release.yml --limit 3` / `gh run watch <id>`;
   at the end `gh release view vX.Y.Z` must list 12 files (2 Windows, 2 macOS, 4 per Linux
   architecture) plus `SHA256SUMS.txt`.

## While the workflow cannot run (10-22 November 2026; only if it happens again)

The packages are made on this Windows PC and Matteo publishes them. v0.5.0 and v0.6.0 were
released this way, with Windows and Linux x86_64 only (macOS and Linux aarch64 need the
workflow).

1. Steps 1-4 above as usual: the WSL build clones the **committed** HEAD.
2. Close VetroScan if it runs from `target\release` (the build cannot replace the exe). Then,
   in the background and at the same time (each takes 2-3 minutes):
   ```bash
   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1
   ```
   ```bash
   MSYS_NO_PATHCONV=1 wsl -d Ubuntu -- bash -lc 'SRC=/mnt/c/Users/white/Desktop/Driver_Software_hp3770/repo OUT=/mnt/c/Users/white/Desktop/Driver_Software_hp3770/repo/dist/vX.Y.Z-linux bash /mnt/c/Users/white/Desktop/Driver_Software_hp3770/repo/scripts/package-wsl.sh'
   ```
   `package.ps1` leaves the setup and the portable zip in `dist/`; `scripts/package-wsl.sh`
   (the CI's pinned tools: keep them in step with `release.yml`) builds and checks the .deb,
   .rpm, .pkg.tar.zst and AppImage and ends with `SMOKE OK`. Check that `hp3770 --version`
   says the new version.
3. Gather the six packages in `dist/vX.Y.Z/` and write the checksums in the CI's format (two
   spaces, no `*`), then check them:
   ```bash
   cd dist/vX.Y.Z && sha256sum vetroscan-vX.Y.Z-* | sed 's/ \*/  /' > SHA256SUMS.txt && sha256sum -c SHA256SUMS.txt
   ```
4. The notes: the CHANGELOG's `## [X.Y.Z]` section without its heading, in
   `dist/vX.Y.Z-notes.md`.
5. **Matteo pushes and publishes** (the auto mode blocks Claude's `git push` and
   `gh release create` here). Give him these, one per block, for PowerShell 5.1 (no `&&`,
   files named one by one, no `*`):
   ```bash
   git -C C:\Users\white\Desktop\Driver_Software_hp3770\repo push
   git -C C:\Users\white\Desktop\Driver_Software_hp3770\repo tag vX.Y.Z
   git -C C:\Users\white\Desktop\Driver_Software_hp3770\repo push origin vX.Y.Z
   cd C:\Users\white\Desktop\Driver_Software_hp3770\repo\dist\vX.Y.Z
   gh release create vX.Y.Z -R MatteoDM/VetroScan --title vX.Y.Z --notes-file ..\vX.Y.Z-notes.md vetroscan-vX.Y.Z-windows-x86_64-setup.exe vetroscan-vX.Y.Z-windows-x86_64-portable.zip vetroscan-vX.Y.Z-linux-x86_64.deb vetroscan-vX.Y.Z-linux-x86_64.rpm vetroscan-vX.Y.Z-linux-x86_64.pkg.tar.zst vetroscan-vX.Y.Z-linux-x86_64.AppImage SHA256SUMS.txt
   ```
   Afterwards `gh release view vX.Y.Z -R MatteoDM/VetroScan` must list 7 files.

## Installing the new version on this PC

When Matteo asks for it: the installer, silent, per user (no administrator), over the version
already installed, keeping his preferences:
```powershell
Start-Process -FilePath 'dist\vX.Y.Z\vetroscan-vX.Y.Z-windows-x86_64-setup.exe' -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/CURRENTUSER' -Wait -PassThru
```
It goes to `%LOCALAPPDATA%\Programs\VetroScan`; check its `hp3770.exe --version` and the
`DisplayVersion` under `HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall`.

## When a release fails

- **Different version in `prepare`**: the tag does not match `Cargo.toml`. Delete the tag
  only with the user's agreement (`git push origin :refs/tags/vX.Y.Z && git tag -d vX.Y.Z`),
  fix, tag again.
- **A test failed on one system**: fix on `main`, then move the tag to the right commit
  (always with the user's agreement). The `release` job uses `gh release upload --clobber`,
  so running it again on the same tag replaces the files.
- Never change the workflow to skip the `test` job.

## Packaging locally

```bash
scripts/package.sh
```

Without arguments it runs `cargo build --release --locked --bin vetroscan --bin hp3770`,
takes the version from `Cargo.toml` (`cargo pkgid -p vetroscan`) and the label from the
machine (`linux-x86_64`, `macos-arm64`, …, the same as the CI). The CI passes the three
arguments and builds first: `scripts/package.sh <target|""> <label> <version>`. On Linux it
makes `.deb`, `.rpm`, `.pkg.tar.zst` and `.AppImage`; it needs `nfpm` and `appimagetool` in
the `PATH` or in the `NFPM` / `APPIMAGETOOL` variables, and `APPIMAGE_RUNTIME` if
appimagetool cannot download the runtime. On macOS it makes `VetroScan.app`
(Info.plist, icon made with `sips`/`iconutil`, ad-hoc signature, not notarised) inside a
`.dmg`. Every system needs cargo-about (`cargo install --locked --features cli cargo-about`)
for `THIRD_PARTY_LICENSES.txt` (`scripts/third_party_licenses.sh`). The output goes to
`dist/` (ignored by git). On Windows,
`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1` makes the installer
and the portable zip; the CI uses the same script with `-Target` and `-Version`.

The icons in `packaging/icons` (PNG and `.ico`) are generated from
`packaging/linux/vetroscan.svg`: if you change the SVG, generate them again.
```bash
for s in 32 128 256 512 1024; do rsvg-convert -w $s -h $s packaging/linux/vetroscan.svg -o packaging/icons/vetroscan-$s.png; done
magick packaging/icons/vetroscan-256.png -define icon:auto-resize=256,64,48,32,16 packaging/icons/vetroscan.ico
```
