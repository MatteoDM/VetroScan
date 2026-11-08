# Installare driver e app, sistema per sistema

Lo **HP Scanjet 3770** (USB `03f0:2505`) funziona con un'app (`hp3770-app`) e un comando
(`hp3770`) che parlano direttamente con lo scanner via USB. **Non servono SANE, moduli del
kernel né programmi HP**: il «driver» è dentro l'app. Quello che cambia da un sistema
all'altro è solo **come il sistema lascia aprire lo scanner** a un programma normale.

| Sistema | Cosa serve per lo scanner | Pacchetto da scaricare |
|---|---|---|
| Arch, Omarchy, Manjaro | regola udev (inclusa nel pacchetto) | `.pkg.tar.zst` |
| Debian, Ubuntu, Mint | regola udev (inclusa nel pacchetto) | `.deb` |
| Fedora, openSUSE | regola udev (inclusa nel pacchetto) | `.rpm` |
| Qualunque Linux | regola udev (la installa l'app con un pulsante) | `.AppImage` |
| Windows 10 e 11 (x64) | il driver HP già installato, **oppure** WinUSB | `-setup.exe` o `-portable.zip` |
| macOS 11 o più recente | niente | `.dmg` |

Tutti i file sono nella pagina
[Releases](https://github.com/MatteoDM/scanner/releases/latest), con nomi come
`hp3770-v0.1.0-linux-x86_64.deb`. `SHA256SUMS.txt` ha le impronte per controllarli.

**Prima di tutto, su ogni sistema:** collega lo scanner alla corrente e, con il cavo USB,
direttamente al computer (un hub va bene solo se ha il suo alimentatore). Un solo programma
alla volta può usare lo scanner.

- [Linux](#linux)
  - [Arch / Omarchy](#arch--omarchy)
  - [Debian / Ubuntu](#debian--ubuntu)
  - [Fedora / openSUSE](#fedora--opensuse)
  - [AppImage (qualunque Linux)](#appimage-qualunque-linux)
  - [Dal codice](#dal-codice)
- [Windows](#windows)
- [macOS](#macos)
- [Dove sono le impostazioni](#dove-sono-le-impostazioni)

---

## Linux

Su Linux un dispositivo USB appena collegato appartiene a root. La **regola udev**
`70-hp3770.rules` lo dà all'utente seduto davanti al computer (quello della sessione grafica,
non uno collegato via ssh):

```text
SUBSYSTEM=="usb", ATTR{idVendor}=="03f0", ATTR{idProduct}=="2505", MODE="0660", TAG+="uaccess"
```

I pacchetti `.pkg.tar.zst`, `.deb` e `.rpm` la mettono in `/usr/lib/udev/rules.d/` e la
applicano subito, anche allo scanner già collegato: **non serve fare altro**.

Prima di scaricare, controlla l'architettura del computer:

```bash
uname -m
```

`x86_64` → file `…-linux-x86_64…`; `aarch64` → file `…-linux-aarch64…`.

Cosa installano i pacchetti:

| File | Cos'è |
|---|---|
| `/usr/bin/hp3770-app` | l'app |
| `/usr/bin/hp3770` | il comando |
| `/usr/lib/udev/rules.d/70-hp3770.rules` | il permesso di usare lo scanner |
| `/usr/share/applications/hp3770-app.desktop` | la voce nel menu delle applicazioni |
| `/usr/share/icons/hicolor/…/hp3770-app.*` | l'icona |
| `/usr/share/licenses/hp3770/` | licenza e licenze delle librerie |

### Arch / Omarchy

1. Scarica `hp3770-vX.Y.Z-linux-x86_64.pkg.tar.zst` nella cartella Scaricati.
2. Apri un terminale (in Omarchy **Super + Invio**) e installa:
   ```bash
   cd ~/Downloads
   ```
   ```bash
   sudo pacman -U hp3770-v*-linux-x86_64.pkg.tar.zst
   ```
   Scrivi la password del computer (mentre la scrivi non compare niente) e conferma con
   **Invio**.
3. Controlla (vedi [Controllare che funzioni](#controllare-che-funzioni-linux)).
4. Apri l'app dal lanciatore (in Omarchy **Super + Spazio**, scrivi «HP Scanjet»).

Se avevi installato dal codice con `cargo install` o il pacchetto AUR `hp3770-git`, toglili
prima: `cargo uninstall hp3770-app hp3770-cli` oppure `sudo pacman -R hp3770-git`.

**Aggiornare:** scarica il pacchetto nuovo e ripeti il passo 2.
**Disinstallare:** `sudo pacman -R hp3770`

### Debian / Ubuntu

1. Scarica `hp3770-vX.Y.Z-linux-x86_64.deb`.
2. Nel terminale:
   ```bash
   cd ~/Scaricati
   ```
   ```bash
   sudo apt install ./hp3770-v*-linux-x86_64.deb
   ```
   Il `./` davanti al nome è necessario: dice ad `apt` che è un file, non un pacchetto da
   cercare in rete. `apt` scarica da solo quello che manca (librerie grafiche, portale per
   «Salva», CUPS per stampare).
3. Controlla e apri l'app dal menu delle applicazioni.

**Aggiornare:** ripeti il passo 2 con il file nuovo.
**Disinstallare:** `sudo apt remove hp3770`

### Fedora / openSUSE

1. Scarica `hp3770-vX.Y.Z-linux-x86_64.rpm`.
2. Nel terminale, dalla cartella in cui l'hai scaricato:
   - Fedora:
     ```bash
     sudo dnf install ./hp3770-v*-linux-x86_64.rpm
     ```
   - openSUSE:
     ```bash
     sudo zypper install ./hp3770-v*-linux-x86_64.rpm
     ```
     `zypper` avvisa che il pacchetto non è firmato: rispondi **i** (ignora) per installarlo.
3. Controlla e apri l'app dal menu delle applicazioni.

**Aggiornare:** ripeti il passo 2 con il file nuovo.
**Disinstallare:** `sudo dnf remove hp3770` oppure `sudo zypper remove hp3770`

### AppImage (qualunque Linux)

Un solo file che si avvia senza installare niente. Non può installare la regola udev da
solo: lo fa l'app, una volta, con un pulsante.

1. Scarica `hp3770-vX.Y.Z-linux-x86_64.AppImage`, per esempio in `~/Applicazioni`.
2. Rendilo eseguibile e avvialo:
   ```bash
   chmod +x ~/Applicazioni/hp3770-v*-linux-x86_64.AppImage
   ```
   ```bash
   ~/Applicazioni/hp3770-v*-linux-x86_64.AppImage
   ```
   Dal file manager: tasto destro → *Proprietà* → *Permessi* → *Consenti l'esecuzione*, poi
   doppio clic.
3. Con lo scanner collegato, la scheda **Scansione** dice «Lo scanner è collegato, ma Linux non
   lo lascia usare a questo utente» e mostra **Permetti l'accesso allo scanner**. Premilo e
   scrivi la password: l'app installa la regola in `/etc/udev/rules.d/70-hp3770.rules`, la
   applica e si ricollega allo scanner, senza staccare il cavo. Serve solo la prima volta.

Il comando `hp3770` è dentro l'AppImage: per usarlo,
`~/Applicazioni/hp3770-v*-linux-x86_64.AppImage --appimage-extract` crea una cartella
`squashfs-root` con `squashfs-root/usr/bin/hp3770`.

Se l'AppImage non parte e parla di **FUSE**, avviala così:
`./hp3770-v*-linux-x86_64.AppImage --appimage-extract-and-run`.

**Aggiornare:** scarica il file nuovo e cancella il vecchio.
**Disinstallare:** cancella il file, poi la regola: `sudo rm /etc/udev/rules.d/70-hp3770.rules`

### Dal codice

Per compilare l'app sul proprio computer, passo per passo su Omarchy:
[installazione-linux.md](installazione-linux.md). Su Arch c'è anche il pacchetto AUR che
compila dall'ultima versione su GitHub: `yay -S hp3770-git`
([PKGBUILD](../packaging/aur/PKGBUILD)).

### Controllare che funzioni (Linux)

Con lo scanner acceso e collegato:

```bash
lsusb | grep 03f0:2505
```

Deve comparire una riga con l'HP. Poi, **senza sudo**:

```bash
hp3770 status
```

Deve rispondere con lo stato di lampada, carrello e pulsanti. Se dice che manca il permesso:

- con un pacchetto: stacca e ricollega il cavo USB; se non basta,
  `sudo udevadm control --reload && sudo udevadm trigger` e ricollega;
- se sei collegato via ssh, la regola non vale (dà lo scanner solo a chi è alla tastiera):
  prova da un terminale della sessione grafica.

### Problemi frequenti (Linux)

- **«Salva…» non apre la finestra per scegliere il file.** Manca il portale del desktop.
  Arch/Omarchy: `sudo pacman -S --needed xdg-desktop-portal-gtk`; Debian/Ubuntu:
  `sudo apt install xdg-desktop-portal-gtk`; Fedora: `sudo dnf install xdg-desktop-portal-gtk`.
  Poi esci dalla sessione e rientra.
- **La fotocopia (tasto Copy) non stampa.** La stampa passa da CUPS (`lp`): configura una
  stampante e impostala come predefinita. Prova con `lp` seguito dal nome di un'immagine.
- **L'app non si apre e parla di Vulkan o OpenGL.** Mancano i driver grafici:
  Debian/Ubuntu `sudo apt install mesa-vulkan-drivers`, Fedora
  `sudo dnf install mesa-vulkan-drivers`, Arch `sudo pacman -S vulkan-icd-loader` più il
  driver della scheda (`vulkan-radeon`, `vulkan-intel` o quello NVIDIA).

---

## Windows

Windows 10 o 11 a 64 bit. L'app parla con lo scanner in uno di due modi, e li prova da sola
in quest'ordine:

1. **Driver HP** (`usbscan.sys`): se lo scanner è già installato con il driver HP, l'app usa
   quello. Non cambia niente nel sistema e le altre app di scansione continuano a funzionare.
2. **WinUSB**: se il driver HP non c'è, si assegna allo scanner il driver generico WinUSB di
   Microsoft con **Zadig**. Da quel momento lo scanner lo usa solo questa app (e il comando
   `hp3770`), non più le app di scansione di Windows.

### 1. Il driver: controlla cosa c'è già

1. Collega e accendi lo scanner.
2. Tasto destro su **Start** → **Gestione dispositivi**.
3. Cerca lo scanner:
   - sotto **Dispositivi di imaging** (o **Fotocamere e scanner**) c'è **HP Scanjet 3770**
     senza triangolo giallo → **il driver HP è installato, non serve altro**. Vai al punto 2.
   - sotto **Altri dispositivi** c'è un dispositivo sconosciuto con il triangolo giallo, o
     niente di HP → **manca il driver**: segui *WinUSB con Zadig* qui sotto.

#### WinUSB con Zadig (solo se manca il driver HP)

1. Scarica Zadig da [zadig.akeo.ie](https://zadig.akeo.ie) e avvialo (chiede la conferma di
   amministratore).
2. Menu **Options** → spunta **List All Devices**.
3. Nel menu a tendina scegli lo scanner: controlla che accanto a **USB ID** ci sia
   **`03F0 2505`**. Attenzione a non scegliere un altro dispositivo (tastiera, mouse, disco):
   cambieresti il suo driver.
4. A destra della freccia verde scegli **WinUSB**.
5. Premi **Install Driver** (o **Replace Driver**) e aspetta il messaggio di conclusione.

Per tornare indietro: **Gestione dispositivi** → tasto destro sullo scanner →
**Disinstalla dispositivo** → spunta *Tenta di rimuovere il driver* → stacca e ricollega il
cavo.

### 2. L'app

**Installer (consigliato)**

1. Scarica `hp3770-vX.Y.Z-windows-x86_64-setup.exe` e avvialo.
2. Se compare **Windows ha protetto il PC** (SmartScreen: l'installer non è firmato con un
   certificato a pagamento), premi **Ulteriori informazioni** → **Esegui comunque**.
3. Scegli **Installa solo per me**: non chiede i diritti di amministratore e mette l'app in
   `%LOCALAPPDATA%\Programs\HP3770`. *Installa per tutti gli utenti* la mette in
   `C:\Program Files\HP3770` e li chiede.
4. Avanti fino alla fine; volendo, spunta l'icona sul desktop. L'app si apre dal menu **Start**
   → **HP Scanjet 3770**.

Installazione silenziosa (per chi la distribuisce):
`hp3770-vX.Y.Z-windows-x86_64-setup.exe /VERYSILENT /CURRENTUSER` (oppure `/ALLUSERS`).

**Portatile (senza installare)**

1. Scarica `hp3770-vX.Y.Z-windows-x86_64-portable.zip`.
2. Tasto destro → **Estrai tutto** in una cartella a scelta.
3. Doppio clic su `hp3770-app.exe`.

### 3. I tasti dello scanner (solo con il driver HP)

Con il driver HP, a ogni pressione di un tasto Windows apre anche la sua app di scansione, che
porta via lo scanner a questa. Nell'app, scheda **Preferenze**, premi **Usa i tasti dello
scanner solo con questa app** e conferma da amministratore una volta: Windows non fa più
niente sui tasti. **Ridai i tasti a Windows** ripristina com'era. Con WinUSB il problema non
c'è.

### Controllare che funzioni (Windows)

In **PowerShell** (Start → scrivi «PowerShell»), con lo scanner collegato:

```powershell
& "$env:LOCALAPPDATA\Programs\HP3770\hp3770.exe" list
```

(Con l'installazione per tutti gli utenti: `& "C:\Program Files\HP3770\hp3770.exe" list`;
con lo zip portatile, il percorso della cartella estratta.) Elenca lo scanner con il backend
che lo apre: `usbscan.sys` con il driver HP, `nusb` con WinUSB. Se `nusb` dice
`bound to …, nusb needs WinUSB`, lo scanner ha un altro driver: ripeti Zadig. Poi:

```powershell
& "$env:LOCALAPPDATA\Programs\HP3770\hp3770.exe" status
```

### Aggiornare e disinstallare (Windows)

- **Aggiornare:** avvia il `-setup.exe` della versione nuova; sostituisce la vecchia e tiene
  le impostazioni.
- **Disinstallare:** **Impostazioni** → **App** → **App installate** → **HP Scanjet 3770** →
  **⋯** → **Disinstalla**. Lo zip portatile si cancella e basta. Il driver (HP o WinUSB)
  resta: per toglierlo, vedi *Per tornare indietro* più sopra.
- **La fotocopia** (tasto Copy) stampa sulla stampante predefinita di Windows attraverso
  Paint.

---

## macOS

macOS 11 o più recente, Apple Silicon (M1 e successivi) o Intel. Non serve nessun driver:
l'app apre lo scanner direttamente.

> **Ancora da provare.** L'app per macOS viene compilata a ogni versione, ma non è ancora
> stata provata con uno scanner vero. Se qualcosa non va, apri una segnalazione su GitHub con
> l'uscita di `hp3770 list` e `hp3770 status`.

1. Scopri il processore:  → **Informazioni su questo Mac**. **Chip Apple M…** → scarica
   `hp3770-vX.Y.Z-macos-arm64.dmg`; **Processore Intel** → `hp3770-vX.Y.Z-macos-x86_64.dmg`.
2. Apri il `.dmg` e trascina **HP Scanjet 3770** sulla cartella **Applications** (oppure in
   `Applicazioni` della tua cartella Inizio, che non chiede la password).
3. **Prima apertura.** L'app non è firmata con un certificato Apple a pagamento, quindi macOS
   dice che non può verificarla:
   - macOS 15 e successivi: premi **Fine**, apri **Impostazioni di Sistema** →
     **Privacy e sicurezza**, scorri fino a *Sicurezza* e premi **Apri comunque** accanto a
     HP Scanjet 3770, poi conferma con la password. Serve una volta sola.
   - macOS 14 e precedenti: in **Applicazioni** tasto destro (o Ctrl+clic) sull'app →
     **Apri** → **Apri**.
   - Dal Terminale, lo stesso effetto:
     ```bash
     xattr -dr com.apple.quarantine "/Applications/HP Scanjet 3770.app"
     ```
4. Se macOS chiede di permettere all'app di accedere a un **accessorio USB**, premi
   **Consenti**.

**Il comando `hp3770`** è dentro l'app. Per averlo nel Terminale:

```bash
sudo mkdir -p /usr/local/bin
```
```bash
sudo ln -sf "/Applications/HP Scanjet 3770.app/Contents/MacOS/hp3770" /usr/local/bin/hp3770
```

Poi, con lo scanner collegato, `hp3770 list` e `hp3770 status`.

- **Aggiornare:** scarica il `.dmg` nuovo e trascina l'app sostituendo la vecchia.
- **Disinstallare:** trascina **HP Scanjet 3770** da Applicazioni nel Cestino, e
  `sudo rm /usr/local/bin/hp3770` se avevi creato il collegamento.
- **La fotocopia** stampa sulla stampante predefinita del Mac.

---

## Dove sono le impostazioni

Preferenze e preset restano tra un aggiornamento e l'altro, e non vengono cancellati
disinstallando:

| Sistema | Cartella |
|---|---|
| Linux | `~/.config/hp3770` |
| Windows | `%APPDATA%\hp3770` |
| macOS | `~/Library/Application Support/hp3770` |

Per ripartire da zero, chiudi l'app e cancella quella cartella.
