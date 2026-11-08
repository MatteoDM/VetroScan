# Novità

Tutte le modifiche che contano per chi usa lo scanner, versione per versione.

Il formato segue [Keep a Changelog](https://keepachangelog.com/it-IT/1.1.0/) e i numeri di
versione il [Versionamento Semantico](https://semver.org/lang/it/).

Ogni modifica visibile a chi usa l'app o il comando `hp3770` va sotto `[Unreleased]` nello
stesso commit che la fa, in una tra *Added*, *Changed*, *Deprecated*, *Removed*, *Fixed* e
*Security* (i nomi restano in inglese: li legge il flusso di rilascio). Al rilascio la sezione
diventa `## [X.Y.Z] - AAAA-MM-GG` e si aggiornano i link in fondo: il workflow di rilascio
pubblica la sezione con il numero del tag come note della release su GitHub, e si ferma se
manca.

## [Unreleased]

### Added
- **App desktop** `hp3770-app` per Linux e Windows (su macOS si compila, ma non è ancora
  stata provata con lo scanner), con due modi di lavoro:
  - **Documento**: una o più pagine, salvate in PDF, TIFF multipagina o un'immagine per
    pagina;
  - **Foto**: un'immagine per scansione, anche in RAW (DNG e TIFF a 16 bit lineare, con i
    campioni del chip intatti e le alte luci recuperabili).
- Anteprima del vetro con selezione dell'area (maniglie, spostamento, zoom) e ritaglio
  automatico degli originali appoggiati sul vetro.
- Regolazioni dal vivo che non toccano i dati dello scanner: esposizione, contrasto, curva a
  punti, riduzione del rumore, descreening, soglia adattiva per il bianco e nero; Ctrl+Z,
  doppio clic per tornare al valore predefinito, fino a tre preset salvati e preset di
  fabbrica.
- Scansione a colori, in grigio e in bianco e nero da 200 a 2400 dpi, anche a 16 bit per
  canale, con la stessa calibrazione e la stessa curva tonale del driver HP.
- I tre **tasti dello scanner** (Scan, Copy, Instant Share) funzionano con l'app, anche in
  secondo piano, e si configurano per modo; Copy stampa una fotocopia.
- Tema chiaro, scuro o di sistema; preferenze salvate.
- Comando **`hp3770`**: `list`, `status`, `probe`, `buttons`, `init`, `lamp`, `park`,
  `scan`.
- Linux: regola udev che lascia usare lo scanner senza root, voce nel menu delle
  applicazioni con la sua icona, e nell'app il pulsante **Permetti l'accesso allo scanner**
  quando la regola manca.
- Windows: funziona con il driver HP già installato (`usbscan.sys`), senza cambiare driver,
  oppure con WinUSB.
- Pacchetti per ogni sistema, a ogni versione: installer e zip portatile per Windows, `.dmg`
  per macOS, `.deb`, `.rpm`, pacchetto Arch e AppImage per Linux (x86_64 e aarch64). I
  pacchetti `.deb`, `.rpm` e Arch installano anche la regola udev.

[Unreleased]: https://github.com/MatteoDM/scanner/commits/main
