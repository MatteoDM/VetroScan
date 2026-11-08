# Piano: da «app per l'HP 3770» ad «app per scanner»

Stato: **Fase 0 fatta (v0.1.2); prossima: Fase 1** (piano scritto l'8 novembre 2026, dopo la v0.1.1). Ogni fase si chiude con un
commit, i test verdi e una versione; qui si segna cosa è fatto.

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

## Fase 0 · lavoro in corso → v0.1.2 · fatta

PDF con impaginazione per pagina (margine, posizione, orientamento propri, «Applica a tutte
le pagine»), anteprima di stampa con il bordo rosso del taglio e taglio trascinabile con
«Riempi il foglio». Viene prima perché tocca gli stessi file del refactoring (`main.rs`).

## Fase 1 · separare l'imaging dal driver → v0.2.0

- Nuovo crate **`scan-imaging`**: `Image` e `Format` (oggi in `session`), `transform`,
  `convert`, `detect`, `adjust`.
- `adjust` riceve un valore **`Tone`** (bianco di riferimento, curva dell'host) invece del
  `Profile` intero; lo darà ogni scanner.
- `hp3770-core` diventa il solo driver e per questa fase riesporta i tipi spostati.
- **Fatto quando:** tutti i test verdi; un test «golden» passa la stessa scansione in
  `adjust` prima e dopo e ottiene immagini identiche byte per byte; `hp3770-export` dipende
  solo da `scan-imaging`.

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

## Fase 3 · l'HP 3770 dietro il contratto → v0.2.0

- Backend **`rts8822`** nel driver: `impl Scanner` per chip + profilo.
- Tabella **VID/PID → `Profile`** (per ora solo il 3770); `Capabilities` ricavate dal
  profilo: vetro dalla `Platform`, risoluzioni dalla tabella delle modalità, maschera dei
  tasti.
- La CLI passa dal `Registry`; `probe`, `init`, `lamp` restano solo dove il backend li ha.
- **Fatto quando:** test delle catture identici, smoke test sullo scanner vero verdi.

## Fase 4 · l'app parla solo col contratto → v0.2.0

- `worker.rs` tiene un `Box<dyn Scanner>` scelto dal `Registry`; con più scanner, un menu.
- Via `GLASS_INCHES`, `RESOLUTIONS`, `HP_WHITE`: tutto dalle `Capabilities`. I tasti
  diventano «per id» e le loro azioni si salvano per modello (con migrazione).
- **Fatto quando:** gli e2e di oggi passano invariati e **nuovi e2e** con un secondo scanner
  finto diverso (vetro A3, al massimo 1200 dpi, due tasti, niente lampada) mostrano che l'app
  si adatta da sola.
- **Decisione dell'utente:** il nome dell'app e dei pacchetti, se gli scanner diventano tanti.

## Fase 5 · altri HP con chip RTS88xx → v0.3.0 (sperimentale)

- Profili per HP 3800, 3970, 4070, 4370, G3010, G3110 (e gli altri della famiglia) dai dati
  del backend SANE `hp3900`, con la fonte citata come per il 3770.
- Escono «sperimentali»: servono persone con quegli scanner, un `probe`/`status` in una
  segnalazione o, meglio, una cattura USB del driver HP.
- **Rischio alto:** motore, sensore e calibrazione cambiano per modello e senza hardware non
  si verificano; per questo dopo il refactoring e uno alla volta.

## Fase 6 · altre marche → v0.4.0 e oltre

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
| 1–4 · refactoring | 0.2.0 | medio, coperto dai test | sì |
| 5 · altri HP | 0.3.0 | alto | no: servono tester |
| 6 · altre marche | 0.4.0+ | medio | eSCL sì, con uno scanner di rete |
