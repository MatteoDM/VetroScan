---
name: release
description: Pubblica una versione dello scanner HP 3770 — alza la versione del workspace, sposta le novità del CHANGELOG, verifica, fa il commit, crea il tag vX.Y.Z e lo spinge, così il workflow "CI and Release" compila i pacchetti per Linux/Windows/macOS e pubblica la release su GitHub; spiega anche come pacchettizzare in locale con scripts/package.sh e come rimediare a un rilascio fallito. Usala quando si chiede di rilasciare, pubblicare, fare un tag, alzare la versione o creare i pacchetti.
---

# Rilasciare lo scanner

I pacchetti li fa il workflow `CI and Release` (`.github/workflows/release.yml`). Un rilascio è
solo un aumento di versione più un tag uguale.

Pipeline: `test` (Linux, Windows, macOS) e `lint` (Linux: `cargo fmt --all --check`, clippy
con `-D warnings`) → `prepare` (legge la versione del pacchetto `hp3770-app` con
`cargo metadata`; si ferma se il tag non è esattamente `v<versione>`) → `build` (5 target:
linux x86_64/aarch64, windows x86_64, macOS arm64/x86_64; ognuno pacchettizza e poi prova quello
che ha prodotto) → `release` (solo sui tag `v*`: crea la release su GitHub con le note del
CHANGELOG, carica i file e `SHA256SUMS.txt`).

File di ogni release (ognuno con `hp3770-app` e `hp3770`): Windows `…-windows-x86_64-setup.exe`
(Inno Setup, `packaging/windows/hp3770.iss`; di base installazione per utente senza
amministratore) e `…-windows-x86_64-portable.zip`; macOS `…-macos-{arm64,x86_64}.dmg`; Linux
`…-linux-{x86_64,aarch64}.{deb,rpm,pkg.tar.zst,AppImage}` (nFPM `packaging/linux/nfpm.yaml` +
appimagetool). deb, rpm e pacman installano la regola udev in `/usr/lib/udev/rules.d` e la
applicano con `packaging/linux/postinstall.sh`; l'AppImage non può, e l'app offre il pulsante
per i permessi. Versioni e SHA256 di nfpm, appimagetool e del runtime AppImage sono fissati nel
passo *Install Linux packaging tools*: si aggiornano insieme (gli SHA256 degli asset GitHub si
leggono dal campo `digest` di `gh api repos/<owner>/<repo>/releases/tags/<tag>`). Ogni
pacchetto ha anche `THIRD_PARTY_LICENSES.txt`, scritto da cargo-about (`taiki-e/install-action`,
versione fissata nel job build) da `packaging/about.toml` e `packaging/about.hbs`; i test dei
pacchetti controllano che ci sia. Una dipendenza nuova con una licenza che manca in
`packaging/about.toml` blocca il build: aggiungila lì e in `deny.toml` (workflow
*Dependencies*, `cargo deny check`), solo se è compatibile con GPL-2.0-or-later. Non cambiare
mai l'`AppId` in `hp3770.iss`: Windows lo usa per aggiornare e disinstallare. Un tag con `-`
(es. `v0.2.0-rc.1`) esce come prerelease. `workflow_dispatch` compila i pacchetti senza
pubblicarli.

Il PKGBUILD AUR `packaging/aur/PKGBUILD` (`hp3770-git`) compila da git e non segue i tag: non
va toccato a ogni rilascio.

## Passi

1. Parti da `main` pulito e aggiornato (`git status`, `git pull --ff-only`). Guarda l'ultimo
   tag con `git tag --sort=-v:refname | head -1` e chiedi all'utente quale versione rilasciare
   se non l'ha detto (patch/minor/major secondo semver). La prima release è `v0.1.0`, la
   versione già nel `Cargo.toml`: in quel caso non c'è niente da alzare.
2. Alza `version` sotto `[workspace.package]` nel `Cargo.toml` della radice: tutti i crate la
   ereditano (`hp3770 --version` la legge da lì). In `CHANGELOG.md` rinomina
   `## [Unreleased]` in `## [X.Y.Z] - AAAA-MM-GG`, aggiungi sopra un `## [Unreleased]` vuoto e
   aggiorna i link in fondo (`[Unreleased]` confronta `vX.Y.Z...HEAD`, `[X.Y.Z]` confronta il
   tag precedente con `vX.Y.Z`; per la prima, `[0.1.0]` punta a
   `https://github.com/MatteoDM/scanner/releases/tag/v0.1.0`). Controllala con
   `git log --oneline v<precedente>..HEAD`: ogni modifica visibile all'utente va sotto
   *Added*, *Changed*, *Removed* o *Fixed*, scritta per chi usa lo scanner; le correzioni di
   cose mai rilasciate non si elencano. Il job `release` pubblica quella sezione come note e
   fallisce se manca.
3. Aggiorna il lockfile e verifica:
   ```bash
   cargo build --workspace
   cargo test --workspace --locked
   cargo fmt --all --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   ```
4. Commit: `Rilascio vX.Y.Z` con `Cargo.toml`, `Cargo.lock` e `CHANGELOG.md`.
5. Spingi `main` e **fai la prova generale**: *Actions → CI and Release → Run workflow* su
   `main` (o `gh workflow run release.yml --ref main`). Esegue test, `prepare` e i cinque
   `build` ma non `release`. Solo una prova tutta verde garantisce che il tag pubblicherà: lo
   stadio build gira solo su tag o a mano, quindi una CI verde su un push non dice niente sui
   pacchetti.
6. **Chiedi all'utente prima di creare il tag**: spingere un tag pubblica una release. Poi:
   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```
7. Segui la corsa con `gh run list --workflow release.yml --limit 3` / `gh run watch <id>`;
   alla fine `gh release view vX.Y.Z` deve elencare 12 file (2 Windows, 2 macOS, 4 per
   architettura Linux) più `SHA256SUMS.txt`.

## Quando un rilascio fallisce

- **Versione diversa in `prepare`**: il tag non corrisponde al `Cargo.toml`. Cancella il tag
  solo con l'ok dell'utente (`git push origin :refs/tags/vX.Y.Z && git tag -d vX.Y.Z`),
  correggi, rifai il tag.
- **Test fallito su un sistema**: correggi su `main`, poi sposta il tag sul commit corretto
  (sempre con l'ok dell'utente). Il job `release` usa `gh release upload --clobber`, quindi
  rilanciarlo sullo stesso tag sostituisce i file.
- Non modificare mai il workflow per saltare il job `test`.

## Pacchettizzare in locale

```bash
scripts/package.sh
```

Senza argomenti esegue `cargo build --release --locked --bin hp3770-app --bin hp3770`, prende
la versione dal `Cargo.toml` (`cargo pkgid -p hp3770-app`) e l'etichetta dalla macchina
(`linux-x86_64`, `macos-arm64`, …, le stesse della CI). La CI passa i tre argomenti e compila
prima: `scripts/package.sh <target|""> <etichetta> <versione>`. Su Linux crea `.deb`, `.rpm`,
`.pkg.tar.zst` e `.AppImage`; servono `nfpm` e `appimagetool` nel `PATH` o nelle variabili
`NFPM` / `APPIMAGETOOL`, e `APPIMAGE_RUNTIME` se appimagetool non riesce a scaricare il runtime.
Su macOS crea `HP Scanjet 3770.app` (Info.plist, icona fatta con `sips`/`iconutil`, firma
ad-hoc, non notarizzata) dentro un `.dmg`. Ogni sistema ha bisogno di cargo-about
(`cargo install --locked --features cli cargo-about`) per `THIRD_PARTY_LICENSES.txt`
(`scripts/third_party_licenses.sh`). L'uscita va in `dist/` (ignorata da git). Su Windows,
`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1` crea installer e zip
portatile; la CI usa lo stesso script con `-Target` e `-Version`.

Le icone in `packaging/icons` (PNG e `.ico`) sono generate da `packaging/linux/hp3770-app.svg`:
se cambi l'SVG, rigenerale.
```bash
for s in 32 128 256 512 1024; do rsvg-convert -w $s -h $s packaging/linux/hp3770-app.svg -o packaging/icons/hp3770-app-$s.png; done
magick packaging/icons/hp3770-app-256.png -define icon:auto-resize=256,64,48,32,16 packaging/icons/hp3770-app.ico
```
