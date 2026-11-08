# Piano: da «app per l'HP 3770» ad «app per scanner»

Stato: **Fasi 0–4 fatte (v0.1.2; le Fasi 1–4 escono con la v0.2.0, da pubblicare); prossima: Fase 5**
(piano scritto l'8 novembre 2026, dopo la v0.1.1; aggiornato l'8 novembre 2026 con le lingue,
il nome nuovo e le fasi rimandate). Ogni fase si chiude con un commit, i test verdi e una
versione; qui si segna cosa è fatto.

Ordine: prima il refactoring (Fasi 1–4), poi lingue, progetto in inglese e nome nuovo
(Fasi 5–7), che non chiedono hardware; gli altri scanner (Fasi 8–9) sono **rimandati** finché
non c'è hardware su cui provarli.

## Obiettivo

1. Tutti gli scanner HP con chip Realtek RTS88xx (la famiglia del backend SANE `hp3900`)
   funzionano con la stessa app.
2. L'architettura accoglie scanner di altre marche (eSCL/AirScan, SANE, WIA) senza toccare
   l'app, l'elaborazione delle immagini e l'export.
3. Per chi usa oggi l'HP 3770 non cambia niente: stesse scansioni, byte per byte.

## Da dove si parte (misurato sul codice della v0.1.1)

`hp3770-core` ha circa 9 000 righe, di due tipi mescolati:

| Imaging generico (~2 700 righe) | Driver RTS8822 (~6 300 righe) |
|---|---|
| `transform` (nessuna dipendenza), `convert` (nessuna), `detect` (solo `session::Image`), `adjust` (usa `Profile` solo per il bianco di riferimento, `geometry::host_white`) | `protocol`, `transport`, `setup`, `init`, `lamp`, `motor`, `motor_curves`, `calibration`, `scan`, `scan_modes`, `read`, `arrange`, `session`, `status`, `device`, `profile`, `geometry` |

L'app tocca lo scanner in un punto solo, `crates/hp3770-app/src/worker.rs` (comandi ed
eventi), ed esiste già uno scanner finto (`worker::fake`) usato dai test e2e. I legami da
sciogliere:

| Legame | Dove | Problema |
|---|---|---|
| `HP3770`, `HP_WHITE` | `main.rs`, `adjust` | curva tonale e bianco sono di questo modello |
| `GLASS_INCHES`, `RESOLUTIONS` | `main.rs` (costanti) | vetro e risoluzioni cambiano per modello |
| `device::Button` (59 usi), Scan / Copy / Instant Share | `main.rs`, `worker.rs` | altri modelli hanno altri tasti o nessuno |
| `status::Status` con registri (`link_word`, `chipset_id`) | `main.rs`, `worker.rs` | uno scanner di rete non ha registri |
| `Chip<Scanner>`, `hp3770_usb::open` | `worker.rs` | un solo backend possibile |

## Regole per tutte le fasi

- Nessuna fase cambia il comportamento dell'HP 3770: i test sulle catture del driver HP
  (`hp3770-capture`), i test e2e dell'app e gli smoke test sullo scanner vero
  (`cargo test -p hp3770-cli --test hardware -- --ignored --test-threads 1`) restano verdi.
- Un passo alla volta, ognuno compilabile; le riesportazioni temporanee si tolgono alla fine
  della fase che le ha introdotte.
- Le preferenze già salvate dagli utenti si migrano, con un test per ogni migrazione.
- **Le capacità comandano, sempre** (regola chiesta dall'utente l'8 novembre 2026). Ogni
  parte dell'app legata a una funzione dello scanner esiste solo se le `Capabilities` di
  quello scanner la dichiarano: nell'interfaccia (pannelli, voci, opzioni, scorciatoie,
  testi che la nominano) e nei percorsi del backend e dell'app (comandi, timer, salvataggi).
  Per esempio: niente lampada né timer della lampada senza `lamp`; niente tasti né azioni dei
  tasti senza `buttons`, e solo quelli che ci sono; niente 16 bit né RAW senza `16` in
  `depths`; solo le risoluzioni di `resolutions`; vetro e righelli da `glass_inches`;
  diapositive solo con `transparency`; dettagli tecnici solo quelli di `DeviceStatus`. Le
  preferenze di una funzione assente restano salvate (per quando torna lo scanner che ce
  l'ha) ma non si vedono e non agiscono.
  - Ogni funzione nuova del contratto arriva insieme a: il campo in `Capabilities`, il ramo
    dell'app che ne dipende, un e2e con uno scanner finto che ce l'ha e uno con uno che non
    ce l'ha.
  - Nel codice dell'app le funzioni dello scanner si chiedono in un punto solo (un modulo
    di «cosa offre lo scanner aperto»), non con controlli sparsi: così è facile vedere e
    provare che la regola vale ovunque.

## Fase 0 · lavoro in corso → v0.1.2 · fatta

PDF con impaginazione per pagina (margine, posizione, orientamento propri, «Applica a tutte
le pagine»), anteprima di stampa con il bordo rosso del taglio e taglio trascinabile con
«Riempi il foglio». Viene prima perché tocca gli stessi file del refactoring (`main.rs`).

## Fase 1 · separare l'imaging dal driver → v0.2.0 · fatta

- Nuovo crate **`scan-imaging`**: `Image` e `Format` (oggi in `session`), `transform`,
  `convert`, `detect`, `adjust`.
- `adjust` riceve un valore **`Tone`** (bianco di riferimento, curva dell'host) invece del
  `Profile` intero; lo darà ogni scanner.
- `hp3770-core` diventa il solo driver e per questa fase riesporta i tipi spostati.
- **Fatto quando:** tutti i test verdi; un test «golden» passa la stessa scansione in
  `adjust` prima e dopo e ottiene immagini identiche byte per byte; `hp3770-export` dipende
  solo da `scan-imaging`.

Com'è andata (4 commit, dal test golden in poi):

- In `scan-imaging` è passato anche `Mode` (colore, grigio, bianco e nero): `adjust` ne ha
  bisogno. `session::Request` lo usa da lì.
- `Tone` = bianchi del colore per canale, bianco del grigio, gamma, con `Tone::table` (prima
  `geometry::host_tone`). Il driver lo dà con `Profile::tone()`; la spiegazione della curva
  dell'HP sta su `geometry::HOST_GAMMA`.
- Test golden: `crates/hp3770-core/tests/golden_adjust.rs`, 36 casi (`render`,
  `render_samples` a 16 bit, `render16`; tre modi; regolazioni di base, tutte cambiate,
  soglia adattiva; lineare e no). Valori fissati sulla v0.1.2, uguali su Windows e Linux.
- Le riesportazioni sono già tolte alla fine della fase: app, CLI ed export importano da
  `scan_imaging`. `hp3770-core` resta solo come dipendenza di sviluppo di `hp3770-export`,
  per l'esempio `raw_from_png` (il bianco della carta dell'HP 3770).

## Fase 2 · il contratto «scanner» → v0.2.0

Nuovo crate **`scanner-api`**, che non sa niente di USB né di chip:

- `trait Scanner`: `capabilities()`, `status()`, `scan(&ScanRequest, progress) ->
  ScanResult` (annullabile), `poll_buttons()`, e il facoltativo `lamp_off()`.
- `Capabilities`: modello e produttore, misure del vetro, risoluzioni, modi (colore,
  grigio, bianco e nero), profondità, adattatore per trasparenti sì/no, tasti (id,
  etichetta, azione suggerita), lampada e tempo di riscaldamento, `Tone`.
- `DeviceStatus` generico: pronto o occupato, lampada (se c'è), adattatore (se c'è), più una
  lista libera di dettagli tecnici che l'app mostra senza capirli.
- `Registry`: elenca gli scanner di tutti i backend disponibili e li apre.
- Errori comuni: annullato, scollegato, occupato, altro.
- **Fatto quando:** il trait è documentato e lo scanner finto lo implementa.

Fatta. Com'è andata:

- `scanner-api` dipende solo da `scan-imaging` (per `Image` e `Tone`). Una scansione dà
  sempre i campioni **lineari** a colori (8 bit, e 16 se chiesti): l'app fa già da sé
  regolazioni, grigio e bianco e nero, quindi `ScanRequest` è solo risoluzione, area e
  profondità; `Capabilities::modes` dice cosa lo scanner offre di suo.
- `poll_buttons()` dà i tasti premuti ora (bit `i` = `capabilities().buttons[i]`); chi
  legge riconosce le pressioni. `lamp_off()` ha un'implementazione di base che risponde
  `Unsupported`. `check()` controlla una richiesta contro le capacità.
- Lo scanner finto è `scanner_api::fake`, in due modelli: `flatbed()` (come il 3770) e
  `a3()` (vetro A3, fino a 1200 dpi, solo 8 bit, due tasti, niente lampada), con
  `Controls` per tenere premuti i tasti e staccare il cavo. L'app usa già la sua immagine
  del vetro.

## Fase 3 · l'HP 3770 dietro il contratto → v0.2.0

- Backend **`rts8822`** nel driver: `impl Scanner` per chip + profilo.
- Tabella **VID/PID → `Profile`** (per ora solo il 3770); `Capabilities` ricavate dal
  profilo: vetro dalla `Platform`, risoluzioni dalla tabella delle modalità, maschera dei
  tasti.
- La CLI passa dal `Registry`; `probe`, `init`, `lamp` restano solo dove il backend li ha.
- **Fatto quando:** test delle catture identici, smoke test sullo scanner vero verdi.

Fatta. Com'è andata:

- Il `Profile` ha un `DeviceModel` (produttore, modello, VID/PID, vetro in centesimi di
  pollice, risoluzioni, tasti); `profile::PROFILES` e `profile::for_device(vid, pid)` sono
  la tabella. Vetro e risoluzioni sono quelli dell'HP driver (`geometry::GLASS_INCHES`,
  `RESOLUTIONS`), non ricavabili dalla `Platform`: un test controlla che coincidano.
- `hp3770_core::rts8822::Rts8822<T: Transport>` implementa `Scanner`: chiede a
  `session::scan` esattamente la richiesta che faceva l'app (colore lineare, nessuna
  nitidezza, spostamento veloce) e mette nei dettagli lampada, calibrazione, buffer e i
  pixel anomali della striscia bianca. Un test prova che i campioni lineari escono
  intatti, quindi elaborarli dopo dà le stesse immagini di prima.
- `hp3770_usb::UsbBackend` è il backend del `Registry` (apre come `hp3770_usb::open`, con
  `usbscan.sys` prima su Windows); `UsbscanScanner` ora è `Send`.
- `hp3770 scan` passa dal `Registry` (o dal driver diretto con `--backend` o
  `--no-fast-travel`) ed elabora i campioni con il `Tone` dello scanner; gli altri comandi
  restano del driver.
- Smoke test sullo scanner vero: 5 su 5 prima e dopo.

## Fase 4 · l'app parla solo col contratto → v0.2.0

- `worker.rs` tiene un `Box<dyn Scanner>` scelto dal `Registry`; con più scanner, un menu.
- Via `GLASS_INCHES`, `RESOLUTIONS`, `HP_WHITE`: tutto dalle `Capabilities`. I tasti
  diventano «per id» e le loro azioni si salvano per modello (con migrazione).
- **Fatto quando:** gli e2e di oggi passano invariati e **nuovi e2e** con un secondo scanner
  finto diverso (vetro A3, al massimo 1200 dpi, due tasti, niente lampada) mostrano che l'app
  si adatta da sola, secondo la regola «le capacità comandano»: per ogni capacità un e2e
  controlla che con lo scanner che ce l'ha l'app la offra e con quello che non ce l'ha
  non ne mostri niente.

Fatta. Com'è andata:

- Il thread dello scanner (`worker.rs`) tiene un `Box<dyn Scanner>` aperto dal `Registry`
  (`hp3770_usb::UsbBackend`, o lo scanner finto con `HP3770_FAKE_SCANNER`). Gli e2e
  fanno girare lo **stesso** thread su `scanner_api::fake`: prima avevano un finto a parte.
  Gli eventi portano `DeviceStatus`, `Progress`, `ScanResult` e il tasto come indice;
  `Event::Opened` dà capacità ed elenco degli scanner.
- Prima che uno scanner risponda l'app usa il primo modello che i backend conoscono
  (`Backend::models()`); `Capabilities::placeholder()` se non ne conoscono nessuno.
- Il `Tone` viaggia con i campioni (`Raw::tone`): ogni pagina si disegna con la curva dello
  scanner che l'ha acquisita, e il bianco della carta per raddrizzare e per i RAW è il suo.
- Risoluzione e profondità scelte restano salvate; l'app usa la risoluzione più vicina che
  lo scanner offre (`App::dpi`) e 8 bit se non ne dà 16 (`App::depth`). L'anteprima si fa
  alla risoluzione dello scanner più vicina a 200 dpi e si riporta a 200.
- Tasti per id: le azioni si salvano in `Prefs::buttons` per modello e id; le preferenze
  della v0.1.x (per posizione) passano all'HP Scanjet 3770 con un test di migrazione, e si
  continua a scriverle anche nel vecchio formato per chi torna indietro.
- Le capacità comandano: lampada (preferenze, timer, spegnimento alla chiusura, stato),
  tasti (preferenze, riepilogo, suggerimento della striscia, opzione di Windows solo col
  backend `rts8822`), 16 bit, risoluzioni, vetro e dettagli dello stato vengono tutti da
  `Capabilities` / `DeviceStatus`. Con più scanner, «Usa» nelle Preferenze.
- Nuovi e2e: il piano A4 (lampada, tasti, 16 bit), il piano A3 (niente lampada né 16 bit,
  i suoi due tasti, vetro e risoluzioni suoi, chiusura senza attese), uno scanner senza
  tasti, le azioni dei tasti ricordate per modello passando da uno scanner all'altro. Il
  thread ha test suoi: una pressione arriva una volta e solo per tasti che esistono, niente
  spegnimento della lampada a chi non ce l'ha, si apre lo scanner scelto.
- Smoke test sullo scanner vero: in più `the_registry_opens_the_scanner_like_the_app`
  (registro, stato, tasti, scansione a 16 bit, lampada spenta).
- Il nome dell'app e dei pacchetti cambia nella Fase 7.

## Fase 5 · l'app in italiano e in inglese → v0.3.0

L'italiano resta la lingua di base; l'inglese si sceglie dalle Preferenze, e se il sistema è
in inglese l'app parte in inglese.

- **Preferenza «Lingua»**: *Automatica* (di base), *Italiano*, *English*. Automatica = inglese
  se la lingua del sistema comincia con `en` (crate `sys-locale`, MIT/Apache), altrimenti
  italiano. Si salva in `impostazioni.json`; chi non ce l'ha ancora riceve *Automatica*
  (migrazione con il suo test). Cambiarla ridisegna subito l'interfaccia, senza riavvio.
- **Catalogo dei testi**: un file Fluent per lingua (`crates/hp3770-app/i18n/it.ftl`,
  `en.ftl`), incluso nel binario con `include_str!`, letto con `fluent-bundle`
  (Apache/MIT; da aggiungere a `deny.toml` e `packaging/about.toml` se serve). Fluent
  gestisce plurali («1 pagina» / «3 pagine») e parametri, e i file si traducono senza
  toccare il codice. Nel codice una macro `t!("chiave", n = 3)`.
- **Cosa si traduce**: etichette, pulsanti, menu, suggerimenti, note di stato
  (`self.note`), domande e titoli delle finestre di dialogo (`rfd`), notifiche, errori del
  worker mostrati all'utente, nomi di file proposti («Documento», «Pagina»), metadati del PDF.
  La CLI `hp3770` segue la stessa regola della lingua.
- **Pacchetti**: installer Windows con italiano e inglese (`[Languages]` di Inno Setup),
  `Comment` e `GenericName` del `.desktop` in tutte e due le lingue,
  `CFBundleLocalizations` nell'`Info.plist` del `.app` macOS.
- **Passi**, ognuno con i test verdi: (1) catalogo e macro con il solo italiano, i testi
  spostati a gruppi (barra, pannelli, editor, PDF, dialoghi, note): l'app non cambia di un
  carattere; (2) `en.ftl` completo; (3) preferenza e scelta automatica; (4) pacchetti.
- **Fatto quando:** un test verifica che ogni chiave usata nel codice esista in tutti e due i
  cataloghi e che non ce ne siano di avanzate; un test cerca stringhe scritte a mano nei widget
  (`text("…")` con lettere) e fallisce se ne trova; gli e2e di oggi passano invariati in
  italiano e un gruppo di e2e gira in inglese (avvio, scansione, salvataggio, Preferenze).

## Fase 6 · il progetto in inglese → con la v0.3.0

Documentazione, commit e note di rilascio passano all'inglese; l'app resta italiana di base
(Fase 5).

- **Da quando**: dal primo commit di questa fase tutti i messaggi di commit sono in inglese.
  La storia già pubblicata non si riscrive.
- **Documenti**: `README.md` in inglese, più un `README.it.md` corto per chi usa l'app in
  italiano (installazione e primi passi). In inglese anche `docs/uso.md`,
  `docs/installazione.md`, `docs/installazione-linux.md`, `docs/protocollo-usb.md`, questo
  piano, la skill `.claude/skills/release`, i commenti di `packaging/` e il `PKGBUILD`.
  I nomi dei file diventano inglesi (`docs/usage.md`, `docs/install.md`, …) con i link
  aggiornati ovunque.
- **CHANGELOG**: dalla versione di questa fase in inglese; le sezioni passate si traducono
  anche loro, così il file è in una lingua sola (le note delle release già pubblicate su
  GitHub restano come sono). I titoli `Added`/`Changed`/… sono già in inglese.
- **Codice**: commenti e documentazione sono già in inglese; restano da controllare i
  messaggi degli errori e i testi di log, che vanno in inglese o nel catalogo della Fase 5.
- **Regola scritta**: un `CONTRIBUTING.md` in inglese (lingua di commit, CHANGELOG, test da
  far passare, come aggiungere una traduzione).

## Fase 7 · un nome nuovo, non legato all'HP 3770 → v0.4.0

Con più scanner il nome «HP Scanjet 3770» non descrive più l'app. **Deciso dall'utente
(8 novembre 2026): retroscan.** Si scrive `retroscan` in binari, pacchetti, crate e cartelle,
«RetroScan» dove lo si mostra (titolo, barra, menu). Prima di pubblicare si controlla che sia
libero: crates.io, GitHub, Flathub, AUR, Homebrew, marchi registrati.

- **Cosa cambia nome**: titolo della finestra e nome nella barra, binari (`retroscan` per
  l'app, la CLI resta anche come `hp3770` finché serve), pacchetti
  (`retroscan-vX.Y.Z-<sistema>.<ext>`), `.desktop` e icona, bundle macOS e `.dmg`,
  crate dell'app e dell'export (`retroscan-app`, `retroscan-export`; il driver resta
  `hp3770-core`, o diventa `rts88xx` con la Fase 8), README e documenti.
- **Cosa non deve rompersi**:
  - Windows: **l'`AppId` dell'installer non cambia** (è così che Windows aggiorna e
    disinstalla); cambiano nome mostrato e cartella, e l'installer nuovo toglie la vecchia
    voce di menu.
  - Linux: i pacchetti nuovi dichiarano `replaces`/`conflicts`/`provides: hp3770`, così
    l'aggiornamento sostituisce il vecchio; la regola udev resta valida (cambia solo il nome
    del file, con la vecchia tolta in `postinstall.sh`). AUR: pacchetto nuovo e `hp3770-git`
    che rimanda a quello.
  - Preferenze e preset: la cartella passa da `…/hp3770` a `…/retroscan`; al primo avvio si
    copiano i file vecchi (migrazione con test), senza perdere niente.
  - Il repository GitHub (`MatteoDM/scanner`) è già generico; i link alle release vecchie
    restano validi.
- **Fatto quando:** un'installazione della v0.3.x si aggiorna alla nuova su Windows e su
  Linux (deb e rpm) tenendo preferenze e preset, e i controlli finali qui sotto sono puliti.

## Controllo finale di coerenza (doppio) · dopo la Fase 7

Due passate indipendenti, la seconda solo dopo aver corretto tutto quello che trova la
prima.

1. **Automatica**:
   - `git grep` del nome vecchio («Scanjet», «hp3770», «HP3770») fuori dal driver, dalle
     catture e dalle voci passate del CHANGELOG: nessun risultato non spiegato;
   - `git grep` di parole italiane comuni nei documenti e nei commenti («della», «sono»,
     «questo», …): solo in `README.it.md`, in `it.ftl` e nei testi dell'app;
   - test di completezza dei cataloghi e delle stringhe scritte a mano (Fase 5) verdi;
   - CI completa con i pacchetti dai nomi nuovi; prova di aggiornamento dalla versione
     precedente su Windows e Linux; test di migrazione delle preferenze;
   - tutti i link di README e `docs/` funzionano.
2. **A mano**:
   - giro completo dell'app in italiano e in inglese (avvio, anteprima, scansione, regolazioni,
     editor, PDF, salvataggio, Preferenze, tasti dello scanner), con schermate: niente testo
     tagliato, misto o non tradotto;
   - lettura di README, `docs/`, CHANGELOG e note della release dall'inizio alla fine: stesso
     nome, stessa lingua, stesse parole per le stesse cose;
   - installazione pulita su ogni sistema e controllo di nome, icona e voce di menu.

## Fase 8 · altri HP con chip RTS88xx · rimandata (sperimentale)

Rimandata: senza questi scanner non si prova niente. Riparte quando arriva un tester o un
modello in prestito.

- Profili per HP 3800, 3970, 4070, 4370, G3010, G3110 (e gli altri della famiglia) dai dati
  del backend SANE `hp3900`, con la fonte citata come per il 3770.
- Escono «sperimentali»: servono persone con quegli scanner, un `probe`/`status` in una
  segnalazione o, meglio, una cattura USB del driver HP.
- **Rischio alto:** motore, sensore e calibrazione cambiano per modello e senza hardware non
  si verificano; per questo dopo il refactoring e uno alla volta.

## Fase 9 · altre marche · rimandata

Rimandata come la Fase 8: eSCL si prova con una qualsiasi multifunzione di rete, quindi è la
prima a ripartire appena ce n'è una a disposizione.

In ordine di resa per costo:

1. **eSCL/AirScan**: HTTP e scoperta mDNS, uguale su Linux, Windows e macOS; quasi tutti gli
   scanner e le multifunzione di rete moderni, senza driver.
2. **SANE** (Linux, macOS): libsane caricata a runtime, centinaia di modelli USB.
3. **WIA** (Windows): ripiego per chi ha solo il driver Windows.

Ogni backend implementa solo `trait Scanner`: app, imaging ed export non cambiano.

## Riepilogo

| Fase | Versione | Rischio | Senza hardware nuovo? |
|---|---|---|---|
| 0 · PDF per pagina | 0.1.2 | basso | sì |
| 1 · imaging separato (fatta) | 0.2.0 | medio, coperto dai test | sì |
| 2–4 · refactoring | 0.2.0 | medio, coperto dai test | sì |
| 5 · italiano e inglese | 0.3.0 | basso, coperto dai test | sì |
| 6 · progetto in inglese | con la 0.3.0 | basso | sì |
| 7 · nome nuovo + controllo finale | 0.4.0 | medio: aggiornamenti e preferenze | sì |
| 8 · altri HP (rimandata) | da decidere | alto | no: servono tester |
| 9 · altre marche (rimandata) | da decidere | medio | eSCL sì, con uno scanner di rete |
