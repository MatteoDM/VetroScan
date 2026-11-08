# scanner — driver aperto per HP Scanjet 3770

*English: [README.md](README.md). La documentazione completa del progetto è in inglese.*

Driver e applicazione per lo scanner **HP Scanjet 3770** (USB `03f0:2505`), scritti in Rust,
per Linux, macOS e Windows. Le scansioni sono uguali a quelle del driver HP, a colori, in
grigio e in bianco e nero, da 200 a 2400 dpi, anche a 16 bit per canale.

L'app parla italiano e inglese: nelle **Preferenze**, *Lingua* sceglie fra *Automatica*
(la lingua del sistema), *Italiano* ed *English*.

## Installare

Scarica il file per il tuo sistema dall'[ultima release](https://github.com/MatteoDM/scanner/releases/latest):

| Sistema | File | Come |
|---|---|---|
| **Windows** (x64) | `…-windows-x86_64-setup.exe` | Avvialo e lascia **Installa solo per me**: niente diritti di amministratore. Usa il driver HP se c'è, altrimenti WinUSB. |
| Windows, portatile | `…-windows-x86_64-portable.zip` | Estrai e avvia `hp3770-app.exe`. |
| **macOS** | `…-macos-arm64.dmg` / `…-macos-x86_64.dmg` | Trascina **HP Scanjet 3770** in *Applicazioni* (non ancora provato con lo scanner). |
| **Arch / Omarchy** | `…-linux-x86_64.pkg.tar.zst` | `sudo pacman -U …` |
| **Debian / Ubuntu** | `…-linux-x86_64.deb` | `sudo apt install ./…` |
| **Fedora / openSUSE** | `…-linux-x86_64.rpm` | `sudo dnf install ./…` |
| **Qualunque Linux** | `…-linux-x86_64.AppImage` | `chmod +x` e avvialo. |

I pacchetti Linux installano anche la regola che lascia usare lo scanner senza root;
con l'AppImage l'app offre il pulsante **Permetti l'accesso allo scanner**.

**Avvisi al primo avvio.** L'installer Windows e l'app macOS non hanno una firma a pagamento:
su Windows clic su **Ulteriori informazioni → Esegui comunque**; su macOS clic su **Fine**,
poi *Impostazioni di Sistema → Privacy e sicurezza → Apri comunque*.

## Primi passi

1. Collega lo scanner e apri l'app: in alto a destra deve dire **Pronto**.
2. Scegli **Documento** (pagine, PDF) o **Foto** (immagini).
3. **Anteprima**, trascina sull'anteprima per scegliere l'area (o lascia fare al ritaglio
   automatico), regola a destra.
4. **Scansiona**, poi **Salva**.

I tasti dello scanner funzionano anche con l'app in secondo piano: cosa fanno si sceglie
nelle Preferenze.

La guida completa (in inglese) è in [docs/usage.md](docs/usage.md); l'installazione,
sistema per sistema, in [docs/install.md](docs/install.md). Le novità di ogni versione sono
in [CHANGELOG.md](CHANGELOG.md).

## Licenza

GPL-2.0-or-later. Vedi [LICENSE](LICENSE).
