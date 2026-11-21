# VetroScan — scansioni di documenti, foto e pellicole, con driver aperti

*English: [README.md](README.md). La documentazione completa del progetto è in inglese.*

**VetroScan** è un'app per scansionare, scritta in Rust per Linux, Windows e macOS, con i suoi
driver: parla con lo scanner direttamente via USB, senza SANE né il software del produttore.
Mostra solo quello che lo scanner collegato sa fare. Fino alla v0.2 si chiamava «HP Scanjet
3770»: aggiornando, le impostazioni restano.

| Scanner | Vetro | Pellicole (adattatore nel coperchio) |
|---|---|---|
| **HP Scanjet 3770** (USB `03f0:2505`) | 200–2400 dpi, a colori, in grigio e in bianco e nero, 8 o 16 bit; scansioni uguali a quelle del driver HP | negativi, negativi in bianco e nero e diapositive a 600 e 1200 dpi |
| **Epson Perfection 4490 Photo** (USB `04b8:0119`) | 100–4800 dpi (non 800 e 900), 8 o 16 bit | negativi, negativi in bianco e nero e diapositive da 600 a 4800 dpi; **Digital ICE** (la polvere tolta con l'infrarosso) a 600, 1200 e 2400 dpi |

Provati su Windows e Linux (del 4490, su Linux, il vetro); su macOS l'app si installa ma non è
ancora stata provata con uno scanner.

L'app parla italiano e inglese: nelle **Preferenze**, *Lingua* sceglie fra *Automatica*
(la lingua del sistema), *Italiano* ed *English*.

## Installare

Scarica il file per il tuo sistema dall'[ultima release](https://github.com/MatteoDM/scanner/releases/latest):

| Sistema | File | Come |
|---|---|---|
| **Windows** (x64) | `…-windows-x86_64-setup.exe` | Avvialo e lascia **Installa solo per me**: niente diritti di amministratore. Usa il driver del produttore se c'è, altrimenti WinUSB. |
| Windows, portatile | `…-windows-x86_64-portable.zip` | Estrai e avvia `vetroscan.exe`. |
| **macOS** | `…-macos-arm64.dmg` / `…-macos-x86_64.dmg` | Trascina **VetroScan** in *Applicazioni* (non ancora provato con uno scanner; solo nelle release costruite su GitHub). |
| **Arch / Omarchy** | `…-linux-x86_64.pkg.tar.zst` | `sudo pacman -U …` |
| **Debian / Ubuntu** | `…-linux-x86_64.deb` | `sudo apt install ./…` |
| **Fedora / openSUSE** | `…-linux-x86_64.rpm` | `sudo dnf install ./…` |
| **Qualunque Linux** | `…-linux-x86_64.AppImage` | `chmod +x` e avvialo. |

I pacchetti Linux installano anche le regole che lasciano usare gli scanner senza root;
con l'AppImage l'app offre il pulsante **Permetti l'accesso allo scanner**. Il firmware del
4490 è già dentro l'app: non serve altro.

**Avvisi al primo avvio.** L'installer Windows e l'app macOS non hanno una firma a pagamento:
su Windows clic su **Ulteriori informazioni → Esegui comunque**; su macOS clic su **Fine**,
poi *Impostazioni di Sistema → Privacy e sicurezza → Apri comunque*.

## Primi passi

1. Collega lo scanner e apri l'app: in alto a destra deve dire **Pronto**.
2. Scegli **Documento** (pagine, PDF), **Foto** (immagini) o **Pellicola** (negativi e
   diapositive, con uno scanner che ha l'adattatore).
3. **Anteprima**: l'app trova da sola le foto sul vetro o i fotogrammi nel portapellicole;
   correggili, togli quelli che non vuoi o trascina sull'anteprima per aggiungere un'area.
   Regola a destra.
4. **Scansiona** (una scansione per ogni foto o fotogramma scelto), poi **Salva**.

I tasti dello scanner funzionano anche con l'app in secondo piano: cosa fanno si sceglie
nelle Preferenze.

La guida completa (in inglese) è in [docs/usage.md](docs/usage.md); l'installazione,
sistema per sistema, in [docs/install.md](docs/install.md). Le novità di ogni versione sono
in [CHANGELOG.md](CHANGELOG.md).

## Licenza

GPL-2.0-or-later. Vedi [LICENSE](LICENSE). Il firmware del 4490
(`crates/epson-gt-x750/firmware/esfw54.bin`) è di Epson, non è sotto la GPL: vedi la sezione
*Licence* del [README](README.md#licence).
