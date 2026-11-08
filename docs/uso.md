# Usare l'app HP Scanjet 3770

Questa guida spiega come si usa l'app **HP Scanjet 3770** (`hp3770-app`) per acquisire
documenti e foto: cosa fa ogni pulsante, quale formato scegliere, come correggere una
scansione già fatta e cosa fare quando qualcosa non va. Vale per Linux, Windows e macOS; dove
un sistema si comporta in modo diverso è scritto.

Per installare l'app e lo scanner, vedi prima [installazione.md](installazione.md) (o, per un
computer nuovo con Omarchy, [installazione-linux.md](installazione-linux.md)).

> Su macOS l'app si installa e si apre, ma non è ancora stata provata con uno scanner vero.

- [La prima scansione in un minuto](#la-prima-scansione-in-un-minuto)
- [La finestra](#la-finestra)
- [Documento e Foto](#documento-e-foto)
- [Anteprima, area e ritaglio automatico](#anteprima-area-e-ritaglio-automatico)
- [Risoluzione, colore e profondità](#risoluzione-colore-e-profondità)
- [La scheda Regola](#la-scheda-regola)
- [Le pagine: Rifai, Sposta, Elimina](#le-pagine-rifai-sposta-elimina)
- [Modificare una scansione già fatta](#modificare-una-scansione-già-fatta)
- [Salvare: i formati](#salvare-i-formati)
- [I tasti dello scanner e la fotocopia](#i-tasti-dello-scanner-e-la-fotocopia)
- [Le Preferenze](#le-preferenze)
- [Scorciatoie da tastiera e mouse](#scorciatoie-da-tastiera-e-mouse)
- [Problemi frequenti](#problemi-frequenti)

---

## La prima scansione in un minuto

1. Accendi lo scanner e apri l'app. In alto a destra un pallino verde dice
   **Pronto · lampada spenta** (o **accesa**): lo scanner è collegato.
2. In alto scegli **Documento** (fogli, anche più pagine nello stesso file) oppure **Foto**.
3. Metti l'originale sul vetro **a faccia in giù**, contro l'angolo indicato dalla freccia
   sullo scanner, e chiudi il coperchio.
4. Premi **Anteprima**: dopo qualche secondo vedi tutto il vetro. La prima volta la lampada
   deve scaldarsi e ci vuole un po' di più.
5. Se vuoi solo una parte, **trascina sull'anteprima** un riquadro attorno a quello che ti
   serve. In Foto l'app trova da sola le foto appoggiate sul vetro.
6. Premi il pulsante blu in alto (**Scansiona pagina 1**, **Scansiona foto**…).
7. In basso a destra premi **Salva …** (per esempio **Salva PDF…**), scegli il nome e la
   cartella, e conferma.

Tutto il resto di questa guida è per fare meglio queste stesse cose.

---

## La finestra

| Dove | Cosa c'è |
|---|---|
| **In alto** | il nome dello scanner, i modi **Documento** e **Foto**, lo stato dello scanner, i pulsanti **Anteprima** e **Scansiona…**, la scelta del tema (**Sistema**, **Chiaro**, **Scuro**) |
| **A sinistra** | la colonna delle **pagine** (in Documento sempre, in Foto quando ci sono più foto) |
| **Al centro** | il **visore**: l'anteprima del vetro o la scansione fatta |
| **A destra** | quattro schede: **Scansione**, **Regola**, **Uscita**, **Preferenze**; sotto, il pulsante **Salva …** |
| **In basso** | la barra di stato: collegamento USB, risoluzione e colore, dimensione dell'immagine in pixel, l'ultimo messaggio dell'app |

Lo **stato dello scanner**, in alto a destra:

| Pallino | Scritta | Significa |
|---|---|---|
| grigio | Collegamento… | l'app sta cercando lo scanner |
| verde | Pronto · lampada spenta / accesa | si può scansionare |
| arancione | Al lavoro | è in corso un'anteprima o una scansione |
| rosso | Scanner non trovato | vedi [Problemi frequenti](#scanner-non-trovato) |

Un clic sullo stato lo legge di nuovo dallo scanner.

Sul visore ci sono due pulsanti, **Anteprima** e **Pagina** (o **Foto**), per passare dal vetro
alla scansione fatta. Accanto ci sono **Modifica…** (apre l'[editor](#modificare-una-scansione-già-fatta)),
**Contagocce** e **Togli selezione**; in alto a destra **Prima** / **Dopo**; in basso a destra lo
zoom, con **Adatta** (tutta l'immagine nella finestra) e **1:1** (un pixel dell'immagine per
un pixel dello schermo).

Mentre lo scanner lavora compare un riquadro con la percentuale, la fase (avvio,
riscaldamento della lampada, calibrazione, lettura, ritorno del carrello) e il pulsante
**Annulla**. Durante una scansione non si possono cambiare modo e ordine delle pagine, e non si
può salvare.

Quando una pagina del documento è pronta, una foto è stata salvata da sola o una scansione non
riesce, il sistema mostra anche una **notifica** con il suo suono: utile se sei in piedi
davanti allo scanner.

---

## Documento e Foto

L'app ha due modi di lavorare, scelti in alto. Ognuno ricorda le sue impostazioni
(risoluzione, colore, formato, regolazioni, tasti dello scanner): passando dall'uno all'altro
ritrovi quelle che avevi lasciato.

| | **Documento** | **Foto** |
|---|---|---|
| Per | lettere, contratti, ricevute, riviste, libri | foto stampate, cartoline, disegni |
| Ogni scansione | **aggiunge una pagina** al documento | **sostituisce** la foto precedente |
| Si salva in | un unico PDF o TIFF, oppure un'immagine per pagina | un'immagine per foto, anche RAW |
| Di base | 300 dpi, colore, 8 bit, PDF, ritaglio automatico spento | 600 dpi, colore, 16 bit, JPEG, ritaglio automatico acceso |

Il terzo pulsante, **Pellicola**, è grigio: le diapositive e i negativi (con l'adattatore per
trasparenti) non sono ancora supportati.

**Attenzione:**

- In **Foto** una nuova scansione prende il posto di quella che vedi. Salva prima, oppure
  spunta **Salva da sola ogni foto nella cartella** nella scheda **Uscita**.
- Passando da **Documento** a **Foto** resta solo l'ultima pagina del documento: salva il
  documento prima di cambiare modo.

---

## Anteprima, area e ritaglio automatico

### L'anteprima

**Anteprima** acquisisce tutto il vetro a bassa risoluzione, in pochi secondi. Serve a vedere
dove sta l'originale, a scegliere l'area e a provare le regolazioni prima della scansione vera.
Non è obbligatoria: senza anteprima si acquisisce un foglio A4.

### Scegliere l'area

- **Trascina** sull'anteprima con il tasto sinistro per disegnare un riquadro.
- Per correggerlo, prendi una delle **otto maniglie** (angoli e lati) o trascinalo da dentro per
  spostarlo.
- **Togli selezione** (o **Esc**) lo cancella.

La scheda **Scansione** dice dove comincia l'area e quanto è grande, in millimetri, e quanti
pixel e quanta memoria richiederà. **Senza selezione** si acquisisce il formato **A4**
(210 × 297 mm dall'angolo del vetro).

### Ritaglio automatico

Con **Ritaglio automatico** acceso (scheda **Scansione**, sezione *Area*), dopo l'anteprima
l'app cerca gli originali appoggiati sul vetro: foto, cartoline, biglietti, scontrini, anche
diversi insieme e un po' storti. Ognuno compare contornato sull'anteprima e in un elenco con
le misure e quanto è storto. Alla scansione **ognuno diventa un'immagine a sé, ritagliata e
raddrizzata** (in Documento, una pagina a sé).

- **Escludi**, accanto a un originale dell'elenco, lo lascia fuori.
- **Riconosci di nuovo** ripete la ricerca sull'anteprima (per esempio dopo aver escluso
  qualcosa per sbaglio).
- Una **selezione disegnata a mano ha la precedenza** sul ritaglio automatico: per tornare al
  ritaglio automatico, togli la selezione.
- Un foglio bianco sul coperchio bianco può sfuggire: in quel caso disegna l'area a mano.
- Con più originali l'app li acquisisce in una sola passata, oppure uno alla volta se tutti
  insieme richiederebbero troppa memoria; il pulsante in alto dice quanti ne verranno
  (**Scansiona 3 foto**).

Il ritaglio automatico è acceso di base in Foto e spento in Documento.

### Avvisi sulla dimensione

Se la scansione richiede molta memoria (più di circa 400 MB) la scheda **Scansione** lo dice
in rosso, con i GB e i minuti da aspettarsi. A **2400 dpi** senza selezione avvisa che quella
risoluzione serve per originali piccoli: seleziona l'area.

---

## Risoluzione, colore e profondità

Tutto nella scheda **Scansione**, sezione *Originale*.

### Risoluzione

Si sceglie fra **200, 300, 600, 1200 e 2400 dpi** (punti per pollice). Più dpi = più dettaglio,
ma file più grandi e scansioni più lente.

| Originale | dpi consigliati |
|---|---|
| Documenti da leggere, archiviare, inviare | 200–300 |
| Testo da passare a un programma di riconoscimento (OCR) | 300 |
| Foto da guardare sullo schermo o stampare alla stessa misura | 300–600 |
| Foto da ingrandire, archivio di qualità | 1200 |
| Francobolli, piccoli dettagli (solo con un'area selezionata) | 2400 |

### Colore

- **Colore**: per tutto ciò che ha colori.
- **Grigio**: foto in bianco e nero, documenti con matita o timbri sfumati.
- **Bianco e nero**: solo nero e bianco puri, file piccolissimi; ideale per testo stampato.
  Quanto deve essere scuro un punto per diventare nero lo decide la *soglia* (vedi
  [Regola](#la-scheda-regola)).

Lo scanner legge sempre a colori: il colore scelto si applica dopo. Per questo, anche su una
scansione già fatta, l'[editor](#modificare-una-scansione-già-fatta) può passare da grigio a
colore e viceversa.

### Profondità: 8 o 16 bit

- **8 bit per canale**: come il driver HP. Va bene per i documenti.
- **16 bit per canale**: lo scanner manda 65 536 sfumature per colore invece di 256. Le ombre e
  le sfumature restano lisce anche dopo curve e correzioni forti. Costa più memoria (circa il
  triplo) ed è la scelta di base in Foto.

Le regolazioni lavorano sui campioni a 16 bit anche quando poi salvi in un formato normale
(JPEG, PNG): i 16 bit servono già così. Per tenerli **anche nel file**, scegli in **Uscita**
**PNG 16 bit** o **TIFF 16 bit**, che compaiono solo con la profondità a 16 bit (e non in
Bianco e nero). I formati RAW si acquisiscono sempre a 16 bit, qualunque profondità sia
scelta.

---

## La scheda Regola

Le regolazioni **non toccano i dati dello scanner**: l'app tiene da parte la scansione così
com'è arrivata e ci applica sopra le regolazioni, dal vivo sull'anteprima o sulla pagina, e a
piena risoluzione solo quando salvi. Puoi cambiare idea quante volte vuoi senza perdere
qualità.

### A cosa si applicano

In cima alla scheda c'è scritto cosa stai regolando:

- **PROSSIMA SCANSIONE** (con la vista **Anteprima**): regoli l'anteprima e le impostazioni con
  cui verranno le prossime scansioni. In Documento valgono anche per tutte le pagine che non
  hanno regolazioni proprie.
- **PAGINA 2**, **FOTO**… (con la vista **Pagina** o **Foto**): regoli la scansione mostrata.
  - In **Documento** c'è la casella **Solo questa pagina**. Spenta, le modifiche valgono per
    l'anteprima e per tutte le pagine che seguono le regolazioni comuni; accesa, valgono solo
    per questa pagina, che sotto la miniatura mostra **propria**. Spegnendola la pagina torna
    a seguire le regolazioni comuni.
  - In **Foto** anteprima e foto hanno sempre le stesse regolazioni.

### I controlli

**Preset.** In cima, **Applica un preset…** offre le impostazioni pronte del modo:

| Documento | Foto |
|---|---|
| Testo per OCR (bianco e nero) | Foto standard |
| Documento a colori | Archivio foto (1200 dpi, TIFF) |
| Ricevuta / scontrino | Foto sbiadita (recupero colori) |
| Rivista / giornale | Foto in bianco e nero |
| Archivio (600 dpi, senza perdita) | Foto stampata su rivista |

Un preset cambia risoluzione, colore, profondità, formato e regolazioni (non il ritaglio
automatico né il salvataggio automatico). Per salvare i tuoi, scrivi un nome sotto e premi **Salva** (o Invio):
se ne tengono **fino a tre**, visibili in entrambi i modi. Con lo stesso nome di uno esistente
lo sostituisci; **Elimina** accanto al nome lo toglie. I nomi dei preset di fabbrica non si
possono usare. Un preset applicato si annulla con **Ctrl+Z**.

> Applicato mentre guardi una pagina, il preset cambia solo il colore e le regolazioni:
> risoluzione, profondità e formato restano quelli di prima. Per applicarlo per intero,
> passa prima alla vista **Anteprima**.

**Istogramma.** Mostra quanti punti dell'immagine sono scuri (a sinistra) o chiari (a destra),
e la percentuale di **neri tagliati** e **bianchi tagliati**: se è alta, si perdono dettagli
nelle ombre o nelle luci.

**Base** (non in Bianco e nero):
- **Contagocce neutro**: clicca poi sull'immagine un punto che dovrebbe essere grigio o bianco
  (un muro, un foglio, un cielo coperto): l'app toglie la dominante di colore partendo da lì.
  **Esc** lo annulla.
- **Livelli auto**: sceglie da sola il punto del nero e del bianco.
- Cursori **Temperatura** (freddo ↔ caldo), **Tinta** (verde ↔ magenta), **Esposizione** (in
  stop), **Contrasto**, **Luci**, **Ombre**, **Bianchi**, **Neri**.
- *Livelli*: **Punto del nero**, **Punto del bianco**, **Gamma** (mezzitoni), **Luminosità**.

**Curva** (non in Bianco e nero). Il grafico va dalle ombre (in basso a sinistra) alle luci (in
alto a destra), con l'istogramma dietro. **Clic** aggiunge un punto (fino a 16), **trascina**
per spostarlo, **doppio clic** o trascinalo fuori per toglierlo. In Colore si può lavorare su
tutti i canali (**Tutti**) o su **R**, **G**, **B** separati. Tre curve pronte: **Lineare**,
**Contrasto medio**, **Contrasto forte**.

**Colore** (solo in Colore): **Vividezza** (ravviva i colori spenti e risparmia quelli già
accesi) e **Saturazione** (tutti i colori allo stesso modo).

**Dettaglio**:
- **Nitidezza**: 1,5 è quanto fa il driver HP; 0 la spegne.
- **Riduzione rumore**.
- **Descreening**: toglie il retino di stampa di giornali e riviste (l'effetto moiré, le
  righe ondulate). Scegli il tipo di carta stampata: *Giornale (85 lpi)*, *Rivista (133 lpi)*,
  *Rivista patinata (150 lpi)*, *Stampa d'arte (175 lpi)*.

**Bianco e nero** (solo con il colore Bianco e nero):
- **Soglia automatica**: la sceglie l'app guardando l'immagine.
- **Soglia**: più alta = più nero.
- **Soglia adattiva**: per pagine illuminate in modo non uniforme (un libro che non aderisce al
  vetro, un foglio ingiallito ai bordi).

Ogni gruppo si apre e si chiude con un clic sul titolo, e ha il suo **Ripristina**. Un gruppo che
non serve con il colore scelto resta chiuso e dice perché.

### Cursori e numeri

- **Doppio clic** su un cursore lo riporta al valore di base.
- Il numero accanto a ogni cursore si può scrivere a mano: confermalo con **Invio** (vanno bene
  sia la virgola sia il punto).

### Prima / Dopo

**Prima** (sul visore, in alto a destra, oppure il tasto **\\**) mostra l'immagine senza
regolazioni; **Dopo** di nuovo con. Nell'editor, **Prima** mostra la scansione com'era quando
l'hai aperto.

### Annullare: Ctrl+Z

**Ctrl+Z** (su macOS **⌘Z**) annulla l'ultima modifica a regolazioni, curva, colore e preset;
**Ctrl+Y** o **Ctrl+Shift+Z** la rifà. Trascinare un cursore, anche a lungo, conta come un
passo solo. Si tengono gli ultimi 100 passi.

- Una modifica fatta con **Solo questa pagina** si annulla su quella pagina, anche se nel
  frattempo ne guardi un'altra.
- Cambiando fra Documento e Foto la cronologia riparte da zero.
- Ctrl+Z **non** riporta indietro risoluzione, profondità e formato scelti a mano, né le pagine
  eliminate, spostate o sostituite.

---

## Le pagine: Rifai, Sposta, Elimina

La colonna a sinistra mostra le pagine (o le foto) in ordine, con il loro numero. Accanto al
numero, **propria** vuol dire che la pagina ha regolazioni sue, **✎** che è stata girata,
ritagliata o ridimensionata.

- **Clic** sulla miniatura: mostra quella pagina nel visore.
- **Doppio clic**: la apre nell'[editor](#modificare-una-scansione-già-fatta).
- **↑** / **↓**: la sposta prima o dopo.
- **↻**: la gira di 90° in senso orario.
- **Modifica**: apre l'editor.
- **Elimina**: la toglie. **Non c'è conferma e non si annulla.**
- **Rifai**: la acquisisce di nuovo (vedi sotto).

Spostare, eliminare e rifare non si può durante una scansione.

### Rifai e «Tieni la nuova»

**Rifai** serve quando una pagina è venuta storta, mossa o con il coperchio aperto: rimetti
l'originale sul vetro e premi **Rifai** sotto la sua miniatura.

- Una pagina ritagliata con il **ritaglio automatico** viene riacquisita esattamente nello
  stesso punto del vetro (con un millimetro di margine).
- Le altre pagine si riacquisiscono con l'**area e le impostazioni di adesso** (la selezione
  sull'anteprima, oppure l'A4).

Finita la scansione, il visore mostra la nuova versione con la scritta **Nuova scansione della
pagina N** e due pulsanti:

- **Tieni la nuova**: la pagina vecchia viene sostituita, nello stesso posto del documento.
- **Lascia la vecchia**: la nuova scansione viene buttata.

Mentre la scelta è in sospeso puoi ingrandire la nuova scansione per controllarla. Scegli prima
di fare altro: cliccando un'altra miniatura la nuova scansione viene lasciata senza avviso.

La pagina rifatta riparte da zero: le rotazioni, i ritagli e le regolazioni proprie della
pagina vecchia non passano alla nuova.

### Nuovo documento

Nella scheda **Uscita**, **Nuovo documento** toglie tutte le pagine per cominciarne un altro.
Non chiede conferma: salva prima.

---

## Modificare una scansione già fatta

L'**editor** corregge una pagina o una foto dopo la scansione: colori, rotazione,
raddrizzatura, ritaglio, dimensioni. Come le regolazioni, **non rovina niente**: parte sempre
dalla scansione originale, finché non salvi.

**Aprirlo:** **Modifica** sotto la miniatura, doppio clic sulla miniatura, **Modifica…** sul
visore, oppure il tasto **E** mentre guardi una pagina.

**Chiuderlo:**

- **Applica** (o **Invio**): tiene le modifiche.
- **Scarta** (o **Esc**): rimette la pagina com'era quando hai aperto l'editor.
- Cambiando modo, tornando all'anteprima, facendo una nuova scansione o spostando, eliminando o
  rifacendo una pagina, l'editor si chiude **tenendo** le modifiche.

Sopra il visore c'è la barra **Modifica di pagina N** con **Annulla**, **Ripeti**, **Scarta** e
**Applica**. L'editor ha la sua cronologia: **Ctrl+Z** e **Ctrl+Y** annullano e rifanno solo i
passi fatti al suo interno. **Ripristina originale**, in alto a destra nel pannello, riporta
la pagina com'era appena scansionata (anche questo si annulla con Ctrl+Z).

Il pannello ha tre parti: **Colore**, **Ritaglia**, **Dimensioni**.

### Colore

- **Modo**: Colore, Grigio o Bianco e nero, anche diverso da come era stata acquisita.
- Sotto, tutti i controlli della [scheda Regola](#la-scheda-regola): preset, istogramma,
  bilanciamento con contagocce, cursori, curva, saturazione e vividezza, nitidezza,
  descreening, soglia.
- La pagina segue le regolazioni comuni finché non ne cambi una: da lì le regolazioni sono
  solo sue. **Ripristina colori** la fa tornare alle regolazioni comuni e al colore con cui è
  stata acquisita.

### Ritaglia

- **Ruota**: **90° a sinistra**, **90° a destra**, **180°**; **Specchia** (destra ↔ sinistra) e
  **Capovolgi** (sopra ↔ sotto).
- **Raddrizza**: il cursore **Angolo** va da −45° a +45°, a passi di 0,1°; i pulsanti **−1°**,
  **−0,1°**, **+0,1°**, **+1°** lo spostano di poco; doppio clic sul cursore lo azzera. Sul
  visore compare una griglia: allinea a una riga un bordo o una riga di testo. Finché non scegli
  un ritaglio, gli angoli vuoti lasciati dalla rotazione vengono tagliati da soli.
- **Ritaglio**: trascina sull'immagine per un nuovo riquadro, sposta i bordi con le maniglie,
  oppure tutto il riquadro trascinandolo da dentro. **Proporzioni** lo tiene in una forma fissa:
  *Libere*, *Come l'originale*, *Quadrato 1:1*, *4:3*, *3:2*, *16:9*, *A4 / A3 / A5*,
  *Foto 10 × 15*; per quelle non quadrate un pulsante passa da **verticale** a
  **orizzontale**. Sotto si leggono i pixel e i millimetri del ritaglio. **Tutta l'immagine**
  toglie il ritaglio.
- **Ripristina rotazione e ritaglio** annulla tutto questo, lasciando le dimensioni.

Con la tastiera: **R** gira di 90° a destra, **Shift+R** a sinistra.

### Dimensioni

Per rimpicciolire una foto da mandare per email o adattarla a una misura di stampa.

- **Unità**: *Pixel*, *Percentuale*, *Centimetri* o *Pollici*.
- **Larghezza** e **Altezza**: con **Mantieni le proporzioni** acceso, cambiando una cambia
  anche l'altra.
- **Risoluzione** (dpi): in centimetri e pollici la misura sulla carta resta e cambiando i dpi
  cambiano i pixel; con pixel e percentuale cambiano i dpi scritti nel file.
- Pulsanti rapidi **25 %**, **50 %**, **75 %**, **200 %**.
- Il riquadro **Risultato** dice pixel, centimetri e megapixel finali.
- **Dimensione originale** toglie il ridimensionamento.

L'app ricampiona con il metodo Lanczos, che tiene i dettagli nitidi. Ingrandire non aggiunge
dettagli: per un'immagine più grande e definita, rifai la scansione a più dpi.

Rotazioni, ritaglio, raddrizzatura e dimensioni valgono per **tutti** i formati, anche RAW e
16 bit.

---

## Salvare: i formati

Il formato si sceglie nella scheda **Uscita**; il pulsante in basso a destra cambia nome con lui
(**Salva PDF…**, **Salva JPEG…**). Premendolo si apre la finestra del sistema con un nome già
pronto, per esempio `documento-2026-11-08-153012.pdf`, nella
[cartella delle scansioni](#le-preferenze). Si può cambiare nome e cartella.

- In **Documento** si salvano tutte le pagine.
- In **Foto** si salvano tutte le foto della colonna a sinistra (una sola, se è una).

### Documento

| Formato | Cosa esce | Quando |
|---|---|---|
| **PDF** | un file con tutte le pagine | da mandare, stampare, archiviare: il più comodo |
| **TIFF multipagina** | un file con tutte le pagine, senza perdita | archivio di qualità, programmi di OCR |
| **PNG** | un'immagine per pagina, senza perdita | testo e disegni da usare come immagine |
| **JPEG** | un'immagine per pagina, compressa | pagine a colori da mandare in fretta |
| **TIFF** | un'immagine per pagina, senza perdita | come PNG, per programmi che vogliono TIFF |

Per il PDF si sceglie anche la **Pagina PDF**: *Pagina come la scansione* (la pagina ha la
misura dell'area acquisita), *A4* o *Letter*.

### Foto

| Formato | Quando |
|---|---|
| **JPEG** | per tutto: condividere, guardare, stampare. Il più compatibile |
| **PNG** | senza perdita, per foto da ritoccare ancora |
| **TIFF** | senza perdita, per archivio e programmi di stampa |
| **WebP** | senza perdita come PNG, ma più piccolo; per il web |
| **AVIF** | compresso come JPEG ma molto più piccolo; non tutti i programmi lo aprono |
| **RAW · TIFF 16 bit lineare** | per sviluppare la foto in Photoshop o in un altro programma di fotoritocco |
| **RAW · DNG (Lightroom, Camera Raw)** | per sviluppare la foto in Lightroom o Camera Raw |

### 16 bit e RAW

- **PNG 16 bit** e **TIFF 16 bit** (in entrambi i modi, con la profondità a 16 bit, a colori o in
  grigio): l'immagine finita, **con** tutte le regolazioni, ma con 65 536 sfumature per colore.
  Per chi continua a ritoccare in un programma di fotoritocco senza perdere qualità.
- **RAW** (solo in Foto, solo a colori): i campioni dello scanner così come li legge il chip,
  **senza** regolazioni, con le alte luci intatte. Ritaglio, raddrizzatura, rotazione e
  dimensioni valgono; i colori, le curve e gli altri cursori dell'app servono solo a guardare
  l'anteprima. La foto si "sviluppa" poi in Lightroom, Camera Raw o Photoshop, come la foto
  RAW di una fotocamera. La scansione per un RAW si fa sempre a 16 bit.

Se non sai cosa scegliere: **JPEG** per le foto, **PDF** per i documenti.

### Più pagine o più foto in immagini separate

Con PNG, JPEG, TIFF, WebP e AVIF ogni pagina diventa un file: dal nome scelto, per esempio
`foto.jpg`, escono `foto-01.jpg`, `foto-02.jpg`… nella stessa cartella. Con 16 bit e RAW i file
si chiamano `foto-001.tif`, `foto-002.tif`… Scrivendo un nome che finisce in **`.zip`**, le
pagine finiscono invece tutte in un unico archivio ZIP, come immagini PNG.

Il **Bianco e nero** non si salva in JPEG né in AVIF (rovinerebbero i bordi delle lettere), né
a 16 bit: la scheda **Uscita** non li propone.

### Salvataggio automatico (Foto)

Con **Salva da sola ogni foto nella cartella** (scheda **Uscita**, solo in Foto) ogni foto viene
salvata appena acquisita, nella cartella delle scansioni e con il nome automatico, senza
finestre. La barra in basso e una notifica dicono dove.

---

## I tasti dello scanner e la fotocopia

I tre tasti sul davanti dello scanner — **Scan**, **Copy** e **Instant Share** — funzionano con
l'app **anche quando è in secondo piano** o ridotta a icona. Durante una scansione vengono
ignorati. Cosa fanno dipende dal modo, e si cambia nelle [Preferenze](#le-preferenze):

| Tasto | In **Documento** (di base) | In **Foto** (di base) |
|---|---|---|
| **Scan** | Aggiungi una pagina | Scansiona la foto |
| **Copy** | Rifai l'ultima pagina | Fotocopia (scansiona e stampa) |
| **Instant Share** | Salva il documento e apri la cartella | Apri la cartella |

Altre scelte: in Documento *Apri la cartella* e *Niente*; in Foto *Apri la cartella* e *Niente*.

- **Aggiungi una pagina** / **Scansiona la foto**: come il pulsante **Scansiona** dell'app, con
  l'area e le impostazioni di adesso. Così si acquisisce un documento intero restando davanti
  allo scanner: giri il foglio, premi Scan, e la notifica dice quando la pagina è pronta.
- **Rifai l'ultima pagina**: riacquisisce l'ultima pagina e la **sostituisce subito**, senza
  chiedere «Tieni la nuova».
- **Salva il documento e apri la cartella**: salva tutte le pagine nella cartella delle
  scansioni, nel formato scelto e con il nome automatico, senza finestre; poi apre la cartella.
- **Fotocopia**: acquisisce la foto e la manda alla **stampante predefinita**. Su Windows la
  stampa passa da Paint; su Linux e macOS dal sistema di stampa (CUPS, il comando `lp`). Se il
  ritaglio automatico trova più originali sul vetro, viene stampato solo il primo.

La scheda **Scansione** ricorda in fondo cosa fanno i tasti nel modo attuale, e sotto le pagine
una nota dice quale tasto aggiunge una pagina (o scansiona una foto).

### Windows: i tasti e l'app di Windows

Con il driver HP installato, Windows a ogni pressione di un tasto apre anche la sua app di
scansione, che porta via lo scanner a questa. Nelle **Preferenze**, premi **Usa i tasti dello
scanner solo con questa app** e conferma da amministratore: Windows smette di reagire ai tasti.
**Ridai i tasti a Windows** rimette tutto com'era. Il pulsante compare solo con il driver HP;
con WinUSB il problema non c'è.

### Linux e macOS

Su Linux nessun servizio di sistema si prende i tasti: non c'è niente da impostare. Su macOS non
sono ancora stati provati.

---

## Le Preferenze

La quarta scheda a destra. Ogni cambiamento si salva da solo.

- **All'avvio · Modo**: con quale modo si apre l'app: *L'ultimo usato*, *Documento* o *Foto*.
  Risoluzione, colore, formato, pagina PDF e regolazioni di ogni modo si ritrovano comunque come
  li hai lasciati.
- **Cartella dei file**: dove si salvano le scansioni. **Scegli…** ne indica un'altra; il
  pulsante accanto torna a quella di base, **Scansioni** dentro la cartella **Immagini**:

  | Sistema | Cartella di base |
  |---|---|
  | Linux | `~/Immagini/Scansioni` (o `~/Pictures/Scansioni`, secondo la lingua del sistema) |
  | Windows | `C:\Users\<nome>\Pictures\Scansioni` (in Esplora file: *Immagini › Scansioni*) |
  | macOS | `~/Pictures/Scansioni` (nel Finder: *Immagini › Scansioni*) |

  Se non esiste, l'app la crea la prima volta che salvi.
- **Nomi dei file**: come comincia il nome dei file, separatamente per **Documenti** e **Foto**
  (di base `documento` e `foto`). Dopo vengono data e ora; sotto c'è un esempio.
- **Tasti dello scanner in Documento / in Foto**: cosa fanno **Scan**, **Copy** e
  **Instant Share** nel modo in cui sei. Per impostare l'altro modo, passa prima a quello.
- Su **Windows**, con il driver HP: **Usa i tasti dello scanner solo con questa app** (vedi
  sopra).
- **Impostazioni**: dice in che file sono salvate; **Ripristina impostazioni di fabbrica**
  rimette tutto come al primo avvio (modi, cartella, nomi, tasti, tema). I preset salvati
  restano.

Il **tema** (Sistema, Chiaro, Scuro) si sceglie in alto a destra, nella barra.

Preferenze e preset sono nella cartella delle impostazioni (vedi
[Dove sono le impostazioni](installazione.md#dove-sono-le-impostazioni)): restano dopo un
aggiornamento.

---

## Scorciatoie da tastiera e mouse

Su macOS, al posto di **Ctrl** si usa **⌘ Cmd**. Le scorciatoie non funzionano mentre si scrive in
un campo di testo (un nome, un numero).

| Tasti | Cosa fa |
|---|---|
| **Ctrl+P** | Anteprima |
| **Ctrl+Invio** | Scansiona |
| **Ctrl+S** | Salva |
| **Ctrl+Z** | annulla (nell'editor: annulla nell'editor) |
| **Ctrl+Y** o **Ctrl+Shift+Z** | rifà |
| **F** | adatta l'immagine alla finestra |
| **1** | zoom 1:1 |
| **\\** | Prima / Dopo |
| **I** | contagocce |
| **Esc** | spegne il contagocce, oppure toglie la selezione; nell'editor: **Scarta** |
| **E** | apre l'editor sulla pagina mostrata |
| **Invio** | nell'editor: **Applica** |
| **R** / **Shift+R** | nell'editor: gira di 90° a destra / a sinistra |

| Mouse | Cosa fa |
|---|---|
| **Rotella** | zoom attorno al puntatore |
| **Tasto destro** o **centrale** trascinato | sposta l'immagine |
| **Tasto sinistro** trascinato | sull'anteprima disegna l'area; nell'editor, in *Ritaglia*, il ritaglio; altrove sposta l'immagine |
| **Doppio clic su un cursore** | lo riporta al valore di base |
| **Doppio clic su una miniatura** | apre l'editor |

---

## Problemi frequenti

### «Scanner non trovato»

Il pallino in alto è rosso e la barra in basso dice *Scanner non collegato*.

1. Lo scanner è **acceso** e collegato alla **corrente**? Il cavo USB è inserito bene, meglio
   direttamente nel computer (un hub va bene solo se ha il suo alimentatore)? Prova un'altra
   porta.
2. **Un solo programma alla volta** può usare lo scanner. Chiudi un'altra finestra di questa
   app, il comando `hp3770` in un terminale, e su Windows l'app di scansione di Windows o di HP.
3. Clicca sullo stato in alto per riprovare. L'app riprova anche da sola: quando lo scanner
   torna, il pallino ridiventa verde senza riavviare niente.
4. Se non basta:
   - **Linux**: se la scheda **Scansione** dice *Lo scanner è collegato, ma Linux non lo lascia
     usare a questo utente*, premi **Permetti l'accesso allo scanner** e scrivi la password del
     computer: l'app installa la regola `70-hp3770.rules` e si ricollega da sola, senza
     staccare il cavo. Serve una volta sola. Altrimenti segui
     [Controllare che funzioni (Linux)](installazione.md#controllare-che-funzioni-linux).
   - **Windows**: controlla il driver in *Gestione dispositivi*, come spiegato in
     [installazione.md](installazione.md#1-il-driver-controlla-cosa-cè-già).
   - **macOS**: se il sistema ha chiesto di permettere l'accesso a un accessorio USB, deve
     essere stato concesso.

### Lo scanner si è scollegato durante una scansione

La scansione si ferma e la barra in basso dice **Errore:** seguito da una spiegazione (spesso in
inglese, per esempio *transport error*). Le pagine già acquisite restano. Ricollega lo scanner,
aspetta il pallino verde e ripeti la scansione (per una pagina venuta a metà, usa **Rifai**).

### La scansione è lentissima

- Guarda in basso a sinistra: **USB 1.1** vuol dire una porta o un hub lenti. Collega lo scanner a
  un'altra porta, direttamente al computer.
- 1200 e 2400 dpi su un foglio intero richiedono minuti e molta memoria: seleziona solo l'area
  che serve o abbassa la risoluzione.
- La prima scansione dopo un po' di riposo aspetta che la lampada si scaldi.

### L'anteprima non trova le foto sul vetro

Il ritaglio automatico deve essere acceso (scheda **Scansione**). Gli originali più piccoli di
1,5 cm e i fogli bianchi sul coperchio bianco possono sfuggire: disegna l'area a mano. Se
gli originali si toccano, distanziali di qualche millimetro e rifai l'anteprima.

### I tasti dello scanner non fanno niente

- Durante una scansione sono ignorati (la barra in basso lo dice).
- Controlla nelle **Preferenze** cosa fanno nel modo in cui sei: possono essere su *Niente*.
- **Windows**: se a ogni tasto si apre l'app di Windows, usa **Usa i tasti dello scanner solo
  con questa app**.
- Ogni pressione letta finisce nel file `tasti.log`, nella cartella delle impostazioni: utile
  per una segnalazione.

### La fotocopia non stampa

Serve una **stampante predefinita**. Su Windows impostala in *Impostazioni → Stampanti e
scanner*. Su Linux e macOS la stampa passa da CUPS: la stampante deve
essere configurata e predefinita (prova da terminale `lp` seguito dal nome di un'immagine).

### «Salva» non apre la finestra per scegliere il file (Linux)

Manca il portale del desktop: vedi
[Problemi frequenti (Linux)](installazione.md#problemi-frequenti-linux).

### «Salvataggio non riuscito»

- *bianco e nero: niente 16 bit*: una pagina in Bianco e nero non si salva in PNG o TIFF
  16 bit. Scegli un altro formato, o rimetti la pagina in Grigio o Colore nell'editor.
- *niente da salvare*: non c'è ancora nessuna scansione.
- Altrimenti, di solito, la cartella non è scrivibile o il disco è pieno: scegli un'altra
  cartella.

### «Impostazioni non salvate»

L'app non riesce a scrivere nella cartella delle impostazioni (disco pieno o permessi). Le
scansioni funzionano lo stesso, ma alla prossima apertura ritroverai le impostazioni vecchie.

### Ho cancellato una pagina per sbaglio

Elimina non si annulla: rimetti l'originale sul vetro e acquisiscila di nuovo, poi spostala al
suo posto con **↑** / **↓**.

### Ripartire da zero

**Preferenze → Ripristina impostazioni di fabbrica**. Per cancellare anche i preset, chiudi
l'app e cancella la cartella delle impostazioni
([dove si trova](installazione.md#dove-sono-le-impostazioni)).
