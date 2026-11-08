# Installare lo scanner su un computer nuovo con Omarchy

Guida passo per passo, anche per chi non ha mai usato il terminale. Alla fine lo
**HP Scanjet 3770** funziona con la sua app, che si apre dal menu delle applicazioni
come qualunque altro programma.

Tempo: circa 15 minuti, quasi tutti di attesa mentre il computer compila.

Non servono driver HP, SANE o altri programmi di scansione: l'app parla direttamente
con lo scanner.

> **Metodo provvisorio.** Per ora l'app si compila dal codice, per questo servono Rust e
> qualche minuto di attesa. In arrivo un'AppImage (si scarica e si apre, su qualunque
> Linux) e un pacchetto AUR per Omarchy/Arch (`yay -S hp3770`): con quelli questa guida
> si riduce a tre righe.

---

## Prima di cominciare

- Un computer con **Omarchy** installato e collegato a internet.
- Lo **scanner collegato** alla presa di corrente e, con il cavo USB, al computer.
  Un hub USB va bene, purché sia alimentato.
- Sapere **aprire un terminale**: in Omarchy si apre con **Super + Invio** (Super è il
  tasto con il logo di Windows).

**Come si usa il terminale in questa guida.** Ogni riquadro grigio contiene comandi.
Copiali (seleziona e Ctrl+C), incollali nel terminale con **Ctrl+Shift+V** e premi
**Invio**. Quando un comando chiede la **password**, è quella con cui entri nel
computer: mentre la scrivi non compare niente sullo schermo, è normale. Scrivila e
premi Invio.

Le righe che cominciano con `#` sono spiegazioni, non comandi: se le incolli non
fanno niente.

---

## Passo 1 — Installare Rust (il necessario per compilare)

L'app è scritta in Rust e va compilata sul computer. Omarchy installa Rust con un
solo comando:

```sh
omarchy install dev-env rust
```

Chiede la password e poi scarica per un paio di minuti. Quando ha finito,
**chiudi il terminale e aprine uno nuovo** (Super + Invio): il nuovo terminale trova
i comandi appena installati.

Per controllare:

```sh
cargo --version
```

Deve rispondere con qualcosa come `cargo 1.9x.0`. Se dice `command not found`, chiudi
e riapri il terminale ancora una volta; se non basta, esci dalla sessione e rientra.

> Si può fare anche dal menu di Omarchy: **Super + Spazio → Installa → Sviluppo → Rust**
> (in inglese: Install → Development → Rust). Se la voce Rust è grigia, Rust c'è già.

---

## Passo 2 — Scaricare il programma

```sh
mkdir -p ~/Work
cd ~/Work
git clone https://github.com/MatteoDM/scanner.git
cd scanner
```

Così il programma finisce nella cartella `Work/scanner` dentro la tua cartella
personale.

---

## Passo 3 — Permettere al tuo utente di usare lo scanner

Di base Linux lascia usare le periferiche USB solo all'amministratore. Questa regola
dà accesso allo scanner a chi è seduto al computer, senza password:

```sh
sudo install -m 644 packaging/linux/70-hp3770.rules /etc/udev/rules.d/
sudo udevadm control --reload
```

Poi **scollega il cavo USB dello scanner e ricollegalo**: la regola vale da quel
momento.

---

## Passo 4 — Compilare e installare

```sh
cargo install --path crates/hp3770-app --locked
cargo install --path crates/hp3770-cli --locked
```

Il primo comando costruisce l'app e ci mette **qualche minuto** (5 o più su un
portatile): scorrono tante righe `Compiling …`, va tutto bene. Finisce con
`Installed package …`. Il secondo comando, più veloce, installa `hp3770`, un piccolo
comando per controllare lo scanner dal terminale.

I programmi finiscono in `~/.cargo/bin`, che dopo il Passo 1 il terminale conosce
già.

---

## Passo 5 — Controllare che il computer veda lo scanner

```sh
hp3770 status
```

Se va tutto bene la prima riga dice che lo scanner è stato aperto, e poi compaiono
alcune righe (in inglese) tra cui:

```text
Opened the scanner with nusb at bus 003 address 13
USB speed          2.0
carriage at home   yes
```

I numeri di «bus» e «address» possono essere diversi: non importa.

Se invece compare un errore di **permesso** (`Permission denied`, `Access denied`), la
regola del Passo 3 non è attiva: rifai il Passo 3 e **ricollega il cavo USB**. Se dice
che **non trova lo scanner**, vedi [Problemi](#problemi).

---

## Passo 6 — Mettere l'app nel menu delle applicazioni

```sh
install -Dm 644 packaging/linux/hp3770-app.desktop ~/.local/share/applications/hp3770-app.desktop
```

Ora apri il lanciatore delle app con **Super + Alt + Spazio**, scrivi **scanner**
(o **HP**) e compare **HP Scanjet 3770**. Invio, e l'app si apre.

Finito: da qui in poi il terminale non serve più.

---

## Usare l'app, in breve

1. In alto scegli **Documento** (fogli, anche più pagine in un PDF) oppure **Foto**.
2. Metti l'originale sul vetro, **a faccia in giù**, e chiudi il coperchio.
3. Premi **Anteprima**: dopo pochi secondi vedi il vetro. La prima volta la lampada
   deve scaldarsi e ci vuole un po' di più.
4. **Trascina sull'anteprima** per scegliere l'area da acquisire (se non scegli niente
   acquisisce tutto il vetro). Con **Ritaglio automatico** l'app trova da sola le foto
   appoggiate sul vetro, anche più di una.
5. Premi **Scansiona**. Nel modo Documento ogni scansione aggiunge una pagina.
6. Nella scheda **Uscita** scegli il formato, poi premi il pulsante in basso a destra
   (**Salva PDF…**, **Salva…**) e scegli dove salvare.

I **tasti dello scanner** funzionano anche con l'app aperta in secondo piano:

| Tasto | Cosa fa (si cambia nelle Preferenze) |
| --- | --- |
| Scan (sinistro) | aggiunge una pagina |
| Copy (centrale) | rifà l'ultima pagina, oppure stampa una fotocopia |
| Instant Share (destro) | salva il documento e apre la cartella |

**Per Lightroom o Photoshop** scegli in **Uscita** il formato **RAW · DNG**
(Lightroom, Camera Raw) o **RAW · TIFF 16 bit**. Sono i dati dello scanner senza
ritocchi, alte luci comprese, da sviluppare nel programma di fotoritocco.

---

## Aggiornare a una versione nuova

```sh
cd ~/Work/scanner
git pull
cargo install --path crates/hp3770-app --locked
cargo install --path crates/hp3770-cli --locked
```

Le impostazioni dell'app restano: sono salvate in `~/.config/hp3770`.

---

## Problemi

**L'app dice «Scanner non trovato».**
- Lo scanner è acceso e il cavo è collegato? Prova un'altra porta USB, meglio se
  direttamente sul computer.
- Nel terminale, `lsusb | grep 03f0:2505` deve mostrare una riga. Se non mostra niente,
  il computer non vede proprio lo scanner: cavo, presa o porta.
- Se la riga c'è ma `hp3770 status` dà un errore di permesso, rifai il Passo 3 e
  ricollega il cavo.
- Un solo programma alla volta può usare lo scanner: chiudi altre copie dell'app o un
  `hp3770` lasciato aperto in un terminale.

**Premendo «Salva» non si apre la finestra per scegliere il file.**
Serve il «portale» del desktop, che Omarchy ha già. Se è stato tolto:

```sh
sudo pacman -S --needed xdg-desktop-portal-gtk
```

poi esci dalla sessione e rientra.

**La fotocopia (tasto Copy) non stampa.**
La stampa passa dal sistema di stampa (CUPS): la stampante deve essere configurata e
impostata come predefinita. Prova con `lp -d` seguito dal nome della stampante, oppure
configurala dal menu di Omarchy (**Super + Spazio**).

**`cargo install` si ferma con un errore.**
Di solito manca qualcosa dal Passo 1. Rifai il Passo 1, apri un terminale nuovo e
riprova. Se l'errore resta, copia le ultime 20 righe e chiedi aiuto.

**Il lanciatore (Super + Alt + Spazio) non trova l'app.**
Rifai il Passo 6. Il lanciatore a volte si accorge della nuova app solo dopo un
minuto, oppure dopo essere uscito dalla sessione e rientrato.

---

## Disinstallare

```sh
cargo uninstall hp3770-app
cargo uninstall hp3770-cli
rm ~/.local/share/applications/hp3770-app.desktop
sudo rm /etc/udev/rules.d/70-hp3770.rules
rm -r ~/Work/scanner          # il codice scaricato
rm -r ~/.config/hp3770        # le impostazioni (facoltativo)
```

Le scansioni salvate non vengono toccate.
