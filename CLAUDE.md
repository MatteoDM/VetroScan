# VetroScan

A scanner app for any scanner, born as a free driver for the HP Scanjet G3110/3770 (Realtek
RTS8822). Rust workspace, GPL-2.0-or-later. The phases, the decisions and what was measured
on the hardware are in `docs/architecture-plan.md`; read the phase you work on first.

## Crates

| Crate | What it is |
|---|---|
| `scanner-api` | the contract: `trait Scanner`, `Capabilities`, `ScanRequest`, the made-up scanner (`fake.rs`) |
| `scan-imaging` | algorithms that set themselves from what they measure (film, stripes, chroma, colour…) |
| `hp3770-core` | the 3770's driver: profile, registers, calibration, motor (`profile.rs`, `rts8822.rs`) |
| `hp3770-usb`, `hp3770-capture` | USB access; replay of the HP driver's captures as tests |
| `hp3770-cli` | the `hp3770` command (`tma-scan`, `develop`, `lamp`…) |
| `epson-gt-x750` | the Epson Perfection 4490 (phase 12, scanner not arrived) |
| `vetroscan`, `vetroscan-export` | the iced app; PDF/TIFF/JPEG export |

## Talking and writing

- Matteo writes in Italian: answer in Italian. Code, docs, CHANGELOG and commit messages are in
  English.
- Say what was measured and what is assumed or general knowledge; Matteo checks.
- `git push`, tags and releases are Matteo's: give him the commands, one per line, for
  Windows PowerShell 5.1 (no `&&`, no `*` globs).
- Commit only the files you changed (untracked `probe_*.txt` and others are not yours). Commit
  messages: write them to a file in the scratchpad and `git commit -F` (quotes break `-m`).

## Rules of the project

- **Multi-scanner, always**: VetroScan is an app for any scanner, not the HP 3770's app. The
  app (`vetroscan`), the imaging and the export know no scanner by name: a scanner is a driver
  behind `trait Scanner`, registered in `worker::registry()` (the one place the app names
  drivers), and everything the app does with it comes from what it declares. What only one
  scanner needs becomes a capability, or stays in its driver; never an
  `if backend == …` or a text that names a brand in the app. Before adding a feature, ask how
  it works on a scanner without it and on a different scanner (the Epson 4490 is next; the
  made-up scanners in `fake.rs` stand in for others in the e2e). Scanner-specific facts the app
  needs are declared, as `Capabilities::usb` (the Windows buttons row) and
  `LampInfo::switches_off_after` (the lamp's choices) are. Still from the HP 3770, as generic
  defaults: `scan_imaging::convert::HP_SHARPEN` and `DEFAULT_THRESHOLD` (tuned against HP's
  driver); a new scanner may want its own.
- **Capabilities rule**: everything in the app tied to a function of the scanner exists only if
  its `Capabilities` declare it, asked in one place (`App::caps` and helpers). A new function of
  the contract comes with its `Capabilities` field, the app's branch, and an e2e with a made-up
  scanner that has it and one that does not.
- **Four levels** (phase 13): driver = physics and registers; contract = generic capabilities
  per source; imaging = algorithms from what they measure; app = workflow. No number measured
  on the HP 3770 goes into the app or `scan-imaging`.
- **The real scanner**: never send the 3770 a resolution, mode or motor combination that was
  never run on it (a film preview at 200 dpi once ground the motor). Films only at 600 and
  1200 dpi. Say so before Matteo tries anything new, and guard it in the driver before
  anything moves.
- **Measure before building**: a film or image feature starts from a measurement on real scans
  (scripts and notes in `../captures/tma`, `note.txt`; scans in `../scansioni`), then the
  feature.
- Every change visible to the user goes into `CHANGELOG.md` under `[Unreleased]` in the same
  commit, and into `docs/usage.md` when it changes how the app is used.
- App texts: Italian in the code through `tr()` / `trf()` (the key is the Italian text), English
  in `crates/vetroscan/i18n/en.json`, keys sorted, 2-space indent, UTF-8 not escaped. The i18n
  tests fail on a missing, left-over or untranslated text.
- Saved preferences are migrated, with a test for each migration (`prefs.rs`).

## Checks

```bash
cargo build --workspace
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

- The app's e2e (`crates/vetroscan/src/e2e.rs`, about 120 tests) takes about 5 minutes: run the
  tests in the background and read their real exit code; never pipe cargo through `grep`, it
  hides a failure.
- On the real scanner, only when Matteo has it connected:
  `cargo test -p hp3770-cli --test hardware -- --ignored --test-threads 1`.
- To let Matteo try a change: `cargo build --release`, then start `target\release\vetroscan.exe`
  (close it first, or the build cannot replace the exe).

## Windows machine

- Never rewrite files with PowerShell `Get-Content` / `Set-Content` (5.1 reads UTF-8 as ANSI
  and writes a BOM): use the edit tools, or `sed` / Python from Bash.
- Python is `py -3.13 -I` (numpy and Pillow installed); `python3` is the Store's alias.
- `target/` grows past 30 GB: `cargo clean` when the disk fills (full disk = odd test crashes).
- Releases: the `release` skill (`.claude/skills/release`).
