# Contributing

Thank you for helping. A few rules keep the project coherent.

## Language

- Code, comments, documentation, commit messages and release notes are in **English**.
- The app speaks **Italian** (its first language) and **English**. Italian has no catalog:
  the key of every text is the Italian text itself. English is in
  `crates/hp3770-app/i18n/en.json`.
- `README.it.md` is a short Italian summary for Italian users; everything else is in English.

## Before every commit

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

All three must pass. The tests include the app's e2e (`cargo test -p hp3770-app`), the
comparisons with the HP driver's USB captures and a golden test of the adjustments. With the
scanner plugged in and no other program using it, run the smoke tests too when you touch the
driver:

```bash
cargo test -p hp3770-cli --test hardware -- --ignored --test-threads 1
```

## CHANGELOG

Every change visible to whoever uses the app or the `hp3770` command goes under
`## [Unreleased]` in `CHANGELOG.md`, in the same commit, under *Added*, *Changed*,
*Deprecated*, *Removed*, *Fixed* or *Security*, written for people who use the scanner.

## Adding or changing a text of the app

1. In the code, write the Italian text through `tr("…")`, or `trf("… {name} …", &[("name",
   &value)])` when it has values. Never `format!` a text for people directly.
2. Add its English to `crates/hp3770-app/i18n/en.json` (same `{places}` as the Italian).
3. `cargo test -p hp3770-app i18n` tells you if a text has no English, if an English entry is
   no longer used, or if a text skips the translation. To see the keys the code asks for:
   ```bash
   I18N_KEYS=keys.json cargo test -p hp3770-app i18n::keys -- --ignored
   ```
4. A literal that is not for people (a command, a registry name, a file name) but looks like
   text is marked with `// not translated` on its line or the line before.

## Scanner functions

The app shows and does only what the scanner in use declares in its `Capabilities` (see
`crates/scanner-api`). A new function of the contract comes with its field in
`Capabilities`, the branch of the app that depends on it, and an e2e with a made-up scanner
that has it and one that does not (`scanner_api::fake`). See
[docs/architecture-plan.md](docs/architecture-plan.md).

## Releases

The release procedure is in `.claude/skills/release/SKILL.md`. Releases are published only
with the maintainer's agreement.
