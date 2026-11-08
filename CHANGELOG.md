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
- **Lampada**: l'app la spegne dopo un tempo senza scansioni scelto nelle Preferenze (da
  «appena finita la scansione» a 13 minuti, di base 2) e sempre chiudendosi, invece di
  lasciarla accesa 13 minuti; il timer dello scanner resta come sicurezza.

## [0.1.0] - 2026-11-08

Prima versione.

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
- **Editor delle scansioni già fatte**, in Documento, Foto e nei documenti di più pagine: si
  apre con «Modifica» sotto la miniatura, con un doppio clic su di essa, con «Modifica…» sul
  visore o con il tasto E. Le modifiche si vedono subito e ripartono sempre dai campioni dello
  scanner, quindi non rovinano nulla finché non si salva; «Ripristina originale» torna alla
  scansione com'era. Annulla e Ripeti propri (Ctrl+Z, Ctrl+Y), «Applica» (Invio) o «Scarta»
  (Esc), «Prima/Dopo» confronta con com'era all'apertura.
  - **Colore**: di nuovo tutte le regolazioni (bilanciamento del bianco e contagocce, esposizione,
    contrasto, luminosità, livelli e gamma, curva, saturazione, vividezza, nitidezza, preset) e
    il passaggio a grigio o bianco e nero;
  - **Ritaglia**: rotazione di 90° e 180°, specchiatura e capovolgimento, raddrizzatura fine
    (±45° a passi di 0,1°) con griglia dei terzi e taglio automatico degli angoli vuoti,
    ritaglio con le maniglie e proporzioni fisse (1:1, 4:3, 3:2, 16:9, A4, 10 × 15, come
    l'originale) diritte o coricate;
  - **Dimensioni**: in pixel, percentuale, centimetri o pollici, con i dpi e le proporzioni
    bloccabili, ricampionamento Lanczos; vale anche per i file a 16 bit e RAW.
- Tema chiaro, scuro o di sistema; preferenze salvate.
- **Guida d'uso** dell'app per chi non è tecnico, per Linux, Windows e macOS:
  [docs/uso.md](docs/uso.md), collegata dal README e dalle guide di installazione.
- Comando **`hp3770`**: `list`, `status`, `probe`, `buttons`, `init`, `lamp`, `park`,
  `scan`.
- Linux: regola udev che lascia usare lo scanner senza root, voce nel menu delle
  applicazioni con la sua icona, e nell'app il pulsante **Permetti l'accesso allo scanner**
  quando la regola manca.
- Windows: funziona con il driver HP già installato (`usbscan.sys`), senza cambiare driver,
  oppure con WinUSB.
- Prima di un'azione che butterebbe via scansioni non salvate (passare da Documento a Foto,
  Nuovo documento, una nuova foto, Elimina, chiudere la finestra) l'app chiede: Annulla,
  **Salva…** (salva e poi continua) o l'azione.
- **Duplica** una pagina (anche Ctrl+D): la copia usa la stessa scansione ma si ritaglia, si
  gira e si regola per conto suo, per tenere separate più foto acquisite insieme.
- Le miniature delle pagine si **trascinano** per cambiarne l'ordine.
- **PDF su carta**: foglio A3, A4, A5, Letter, Legal, 10 × 15 o 13 × 18 cm (o come la
  scansione), orientamento automatico, verticale o orizzontale (anche per una sola pagina),
  scansione a dimensione reale, adattata al foglio o a pieno foglio, margine. Il PDF c'è
  anche in modo Foto, per stampare.
- **Dimensione del PDF**: qualità dal 30 al 100%, dimensione stimata prima di salvare e un
  limite in MB che l'app rispetta alleggerendo una pagina alla volta, quella che fa
  risparmiare di più.
- Icona nuova, anche nella finestra e nell'eseguibile Windows.
- Pacchetti per ogni sistema, a ogni versione: installer e zip portatile per Windows, `.dmg`
  per macOS, `.deb`, `.rpm`, pacchetto Arch e AppImage per Linux (x86_64 e aarch64). I
  pacchetti `.deb`, `.rpm` e Arch installano anche la regola udev.

### Fixed
- Due salvataggi nello stesso secondo non si sovrascrivono più: il secondo prende « (2)».
- «Rifai»: il riquadro di avanzamento diceva «Scansione pagina N+1» (la prossima pagina nuova)
  invece della pagina rifatta; ora dice «Nuova scansione della pagina N» e, finita la
  scansione, mostra quella pagina anche nel pannello «Regola».
- In Foto i messaggi di «Rifai», «Tieni la nuova», «Lascia la vecchia» ed «Elimina» parlano di
  foto e non di pagine; il salvataggio non dice più «1 pagine».
- «Annulla» durante una scansione o un'anteprima non è più un errore: niente avviso
  «Scansione non riuscita», la barra dice «Scansione annullata».
- Un originale riconosciuto vicino al bordo destro o in fondo al vetro non fa più fallire la
  scansione o il «Rifai» con «area … outside the glass»: l'area chiesta resta sul vetro.
- Il formato «TIFF» con più pagine o più foto salva un file per pagina, come PNG e JPEG; il
  TIFF unico resta di «TIFF multipagina». JPEG e AVIF tengono la qualità del modo.
- Ctrl+Z dopo il cambio fra Documento e Foto non mette più le impostazioni di un modo
  nell'altro; Ctrl+Z sulle regolazioni proprie di una pagina torna su quella pagina, non su
  quella mostrata in quel momento.
- Cambiando modo, le pagine che seguono le regolazioni comuni si ridisegnano con quelle del
  nuovo modo (prima restavano con le vecchie fino alla modifica successiva); lo stesso dopo
  «Ripristina impostazioni di fabbrica».
- Spostare o eliminare pagine mentre si ridisegnano in background non mette più la miniatura
  di una pagina su un'altra; con una nuova scansione in attesa di scelta, spostare le pagine
  non fa più sostituire quella sbagliata con «Tieni la nuova».
- Durante una scansione non si cambiano più modo né ordine delle pagine (la scansione finiva
  nel posto sbagliato).
- Con l'editor aperto, ruotare un'altra pagina e poi cliccare la miniatura di quella in
  modifica la mostra di nuovo (prima non succedeva niente).
- La nota sotto le pagine nomina il tasto dello scanner che aggiunge davvero una pagina (o
  scansiona una foto), come scelto nelle Preferenze, e sparisce se nessuno lo fa.
- Un preset salvato con il nome di un preset di fabbrica non si poteva più applicare: ora il
  nome viene rifiutato.
- Testi con una fila di spazi in mezzo (nota sul RAW, sui 16 bit e sui tasti in Windows); il
  pulsante della cartella predefinita usa il separatore del sistema.

[Unreleased]: https://github.com/MatteoDM/scanner/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/MatteoDM/scanner/releases/tag/v0.1.0
