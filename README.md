# scanner — driver aperto per HP Scanjet 3770

Driver userspace e applicazione per lo scanner **HP Scanjet 3770** (USB `03f0:2505`),
scritti interamente in Rust, per Linux, macOS e Windows.

Lo scanner usa un ASIC **Realtek RTS8822**, lo stesso pilotato dal backend SANE
`hp3900`. Il driver è una traduzione in Rust della parte di `hp3900` che serve al
3770, completata con i parametri estratti dal driver HP originale e verificata
sulle catture USB del driver HP.

> Stato: scansioni uguali a quelle del driver HP a colori, in grigio e in bianco e nero,
> da 200 a 2400 dpi, anche a 16 bit per canale. Avvio, calibrazione e scansione sono
> verificati byte per byte sulle catture del driver HP. Provato sullo scanner su Windows
> (con il driver HP installato) e su Linux (Arch, Wayland). Mancano ancora il backend
> SANE e l'adattatore per trasparenti; su macOS si compila e si pacchettizza a ogni versione,
> ma non è ancora stato provato con lo scanner.

## Installazione

Scarica il file per il tuo sistema dall'[ultima release](https://github.com/MatteoDM/scanner/releases/latest).
Ogni pacchetto contiene l'app `hp3770-app` e il comando `hp3770`. Le istruzioni complete,
driver compreso, sistema per sistema (installare, controllare, aggiornare, disinstallare,
problemi frequenti): **[docs/installazione.md](docs/installazione.md)**.

| Sistema | File | Come |
|---|---|---|
| **Windows** (x64) | `hp3770-vX.Y.Z-windows-x86_64-setup.exe` | Avvialo e lascia **Installa solo per me**: va in `%LOCALAPPDATA%\Programs\HP3770` con la voce nel menu Start e in *App installate*, **senza diritti di amministratore**. Usa il driver HP se è installato, altrimenti WinUSB. |
| Windows, portatile | `hp3770-vX.Y.Z-windows-x86_64-portable.zip` | Estrai dove vuoi e avvia `hp3770-app.exe`; non installa niente. |
| **macOS** (Apple Silicon / Intel) | `hp3770-vX.Y.Z-macos-arm64.dmg` / `-macos-x86_64.dmg` | Aprilo e trascina **HP Scanjet 3770** in *Applicazioni*. Non ancora provato con lo scanner. |
| **Arch / Omarchy** | `hp3770-vX.Y.Z-linux-<arch>.pkg.tar.zst` | `sudo pacman -U hp3770-vX.Y.Z-linux-x86_64.pkg.tar.zst` |
| **Debian / Ubuntu** | `hp3770-vX.Y.Z-linux-<arch>.deb` | `sudo apt install ./hp3770-vX.Y.Z-linux-x86_64.deb` |
| **Fedora / openSUSE** | `hp3770-vX.Y.Z-linux-<arch>.rpm` | `sudo dnf install ./hp3770-vX.Y.Z-linux-x86_64.rpm` |
| **Qualunque Linux** | `hp3770-vX.Y.Z-linux-<arch>.AppImage` | `chmod +x hp3770-*.AppImage` e avvialo; non installa niente. |

I pacchetti Linux (`x86_64` e `aarch64`) installano anche la regola udev che lascia usare lo
scanner senza root, e la applicano subito, anche a uno scanner già collegato. L'AppImage non
può: al primo avvio l'app offre il pulsante **Permetti l'accesso allo scanner**, che la
installa con la password del computer. Dal codice, su Arch: `yay -S hp3770-git`
(il PKGBUILD è in [`packaging/aur`](packaging/aur)).

**Avvisi al primo avvio.** L'installer Windows e l'app macOS non sono firmati con un
certificato a pagamento: Windows SmartScreen può dire *Windows ha protetto il PC* (clic su
**Ulteriori informazioni → Esegui comunque**); macOS dice che non può verificare l'app: clic
su **Fine**, poi *Impostazioni di Sistema → Privacy e sicurezza → Apri comunque*.

## Usare l'app

Come si usa l'app, per chi non è tecnico: modi Documento e Foto, anteprima e ritaglio
automatico, regolazioni, editor delle scansioni, formati di uscita (RAW e 16 bit compresi),
tasti dello scanner, preferenze, scorciatoie e problemi frequenti:
**[docs/uso.md](docs/uso.md)**.

Le novità di ogni versione sono in [CHANGELOG.md](CHANGELOG.md).

## Struttura

| Crate | Cosa contiene |
| --- | --- |
| [`hp3770-core`](crates/hp3770-core) | Protocollo USB dell'RTS8822 (richieste vendor, registri, EEPROM, DMA), identità del dispositivo, pulsanti, trait `Transport` indipendente dalla libreria USB |
| [`hp3770-capture`](crates/hp3770-capture) | Lettore di catture USBPcap e strumento `hp3770-decode`, che trasforma una cattura in una sequenza leggibile di operazioni sul chip |
| [`hp3770-usb`](crates/hp3770-usb) | Accesso USB: `nusb` (Linux, macOS, Windows con WinUSB) e `usbscan.sys` (Windows con il driver HP installato, nessun cambio di driver) |
| [`hp3770-export`](crates/hp3770-export) | Salvataggio in PNG, JPEG, TIFF (anche a 16 bit e multipagina), WebP, AVIF, DNG, PDF a più pagine e ZIP |
| [`hp3770-cli`](crates/hp3770-cli) | Comando `hp3770` |
| [`hp3770-app`](crates/hp3770-app) | App desktop `hp3770-app` (iced): documenti e foto, anteprima, regolazioni, tasti dello scanner |

In arrivo: backend SANE.

## Compilare e provare

Serve Rust 1.85 o più recente.

```sh
cargo build --release
cargo test
```

I test usano le piccole catture in `crates/hp3770-capture/tests/fixtures`. Le catture
complete delle scansioni (fino a ~27 MB l'una) non sono nel repository: per provare
anche quelle, indica la cartella che le contiene.

```sh
HP3770_CAPTURES=/percorso/catture cargo test
```

## Comando `hp3770`

```sh
cargo run --release --bin hp3770 -- list                     # scanner visti da ogni backend
cargo run --release --bin hp3770 -- status                   # lampada, carrello, adattatore trasparenti, pulsanti
cargo run --release --bin hp3770 -- probe -o probe.txt       # registri ed EEPROM, sola lettura
cargo run --release --bin hp3770 -- buttons                  # stampa i pulsanti premuti (letti dal registro del chip)
cargo run --release --bin hp3770 -- buttons -m interrupt     # oppure dagli eventi USB del pulsante
cargo run --release --bin hp3770 -- init                     # inizializza il chip come il driver HP
cargo run --release --bin hp3770 -- lamp off                 # spegne (o accende, `on`) la lampada del piano
cargo run --release --bin hp3770 -- park                     # riporta a casa il carrello, se non c'è già
cargo run --release --bin hp3770 -- scan foglio.png          # A4 a colori, 300 dpi
cargo run --release --bin hp3770 -- scan foto.png --dpi 1200 --left 80 --top 120 --width 50 --height 50
cargo run --release --bin hp3770 -- scan testo.png --mode lineart   # anche --mode grey
```

`scan` fa tutto quello che fa il driver HP: avvio, riscaldamento della lampada se era spenta,
calibrazione (offset, guadagno, shading nero e bianco), scansione, ritorno del carrello. L'area
è in millimetri dall'angolo in alto a sinistra del vetro (di base un A4). L'immagine esce in PNG
(o PPM/PGM/PBM) con la stessa curva tonale del driver HP; `--linear` salva i dati grezzi del
chip. Non scrive mai l'EEPROM.

`probe` passa da un filtro che rifiuta ogni scrittura prima che arrivi allo scanner:
non accende la lampada, non muove il carrello e non tocca l'EEPROM. Con `--backend nusb`
o `--backend usbscan` si sceglie il backend; di base su Windows si prova prima
`usbscan.sys`.

`init` scrive nei registri del chip gli stessi valori che il driver HP scrive a ogni
pressione di un pulsante (clock USB, tipo di RAM, corrente del motore, reset, pulsanti,
timer della lampada, PWM della lampada). Non scrive mai l'EEPROM e non muove il carrello.

`park` fa `init` e poi, solo se il carrello non è a casa, lo riporta indietro con la stessa
tabella dei passi e lo stesso blocco di registri che il driver HP usa nelle catture 05, 06
e 11 (i test li confrontano byte per byte). Se il carrello è già a casa non lo muove.

`buttons` di base legge il registro dei pulsanti del chip ogni 50 ms: su Windows il servizio
immagini (WIA) resta in ascolto degli eventi USB dei pulsanti e se ne prende una parte,
mentre il registro risponde a chiunque lo legga.

## Linux

Guida passo per passo per un computer nuovo con Omarchy, anche per chi non ha mai
usato il terminale: [docs/installazione-linux.md](docs/installazione-linux.md).

Il driver parla allo scanner direttamente via USB (`nusb`), senza SANE né moduli del
kernel. Serve solo che l'utente possa aprire il dispositivo: la regola udev in
[`packaging/linux`](packaging/linux) lo concede a chi è collegato alla sessione.

```sh
sudo install -m 644 packaging/linux/70-hp3770.rules /etc/udev/rules.d/
sudo udevadm control --reload
# poi scollega e ricollega lo scanner
cargo run --release --bin hp3770 -- status   # deve rispondere senza sudo
```

Per avere l'app nel menu delle applicazioni:

```sh
cargo install --path crates/hp3770-app --locked     # in ~/.cargo/bin, che deve essere nel PATH
install -Dm 644 packaging/linux/hp3770-app.desktop ~/.local/share/applications/hp3770-app.desktop
install -Dm 644 packaging/linux/hp3770-app.svg ~/.local/share/icons/hicolor/scalable/apps/hp3770-app.svg
```

L'app su Linux:

- gira su Wayland e su X11; la finestra si presenta come `hp3770-app`, lo stesso nome
  del file `.desktop`;
- per scegliere dove salvare usa il portale del desktop (`xdg-desktop-portal` con un
  backend, per esempio `xdg-desktop-portal-gtk`);
- apre le cartelle con `xdg-open` e stampa le fotocopie con `lp` (CUPS);
- tiene le preferenze in `~/.config/hp3770`;
- i tasti dello scanner funzionano anche con l'app in secondo piano: li legge dal chip,
  nessun servizio di sistema se li prende come fa WIA su Windows.

## Decodificare una cattura

```sh
cargo run --release --bin hp3770-decode -- cattura.pcap          # tutte le operazioni
cargo run --release --bin hp3770-decode -- --writes cattura.pcap # solo ciò che modifica lo scanner
cargo run --release --bin hp3770-decode -- --summary *.pcap      # totali per file
cargo run --release --bin hp3770-decode -- --executions cattura.pcap
    # registri del chip a ogni avvio (bit 7 di E800): il primo completo, poi le differenze
cargo run --release --bin hp3770-decode -- --slice 2.48 2.623 piccola.pcap cattura.pcap
    # ritaglia un intervallo di tempo in una cattura più piccola (per i test)
```

Nei test, `hp3770_capture::replay::Replay` fa da finto scanner: risponde con i dati
della cattura e controlla che il nostro driver faccia esattamente le stesse richieste
del driver HP, fermandosi alla prima differenza.

Esempio di output:

```text
    16.275  INTR  ep83 00  (button Scan)
    17.316  READ  reg FE11 len 2 -> 03 0A
    17.331  WRITE reg F8FF = 01 10
     2.255  DMA   enable read  channel 0004 options 000000 size 512
```

Le catture si fanno su Windows con il driver HP installato, usando USBPcap e
salvando in formato pcap classico (non pcapng). Il protocollo è descritto in
[docs/protocollo-usb.md](docs/protocollo-usb.md).

## Licenza

GPL-2.0-or-later, come il backend SANE `hp3900` da cui deriva la logica del chip.
Vedi [LICENSE](LICENSE).
