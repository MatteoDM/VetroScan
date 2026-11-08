# Protocollo USB dell'HP Scanjet 3770

Riassunto di ciò che è confermato dalle catture USB del driver HP originale
(Windows 11, driver HP 8.0.0.0, catture del 7 novembre 2026) e dal backend SANE
`hp3900` (`hp3900_usb.c`, `hp3900_rts8822.c`).

## Dispositivo

| Voce | Valore |
| --- | --- |
| Vendor / Product | `03f0` / `2505` |
| ASIC | Realtek RTS8822 (sigla esatta da leggere sul chip) |
| Interfaccia | 1 interfaccia, classe vendor (`FF`), 3 endpoint |
| Endpoint `0x81` | bulk IN: dati immagine e calibrazione |
| Endpoint `0x02` | bulk OUT: tabelle (gamma, shading) |
| Endpoint `0x83` | interrupt IN: pulsanti, 1 byte |
| Firmware | interno all'ASIC, nessun caricamento |

## Richieste di controllo

Tutte le operazioni sul chip sono richieste vendor su EP0 con `bRequest = 0x04`.
`wValue` è l'indirizzo, `wIndex` sceglie l'operazione.

| `bmRequestType` | `wIndex` | Operazione | Dati |
| --- | --- | --- | --- |
| `0xC0` | `0x0100` | lettura registri da `wValue` | `wLength` byte restituiti |
| `0x40` | `0x0000` | scrittura registri da `wValue` | i byte da scrivere |
| `0xC0` / `0x40` | `0x0200` | lettura / scrittura EEPROM | parole a 16 bit |
| `0x40` | `0x0400` | abilita DMA in lettura (verso l'host) | 6 byte |
| `0x40` | `0x0401` | abilita DMA in scrittura (verso lo scanner) | 6 byte |
| `0x40` | `0x0600` | annulla DMA | 2 byte a zero |
| `0x40` | `0x0800` | parola di controllo DMA (`0` = reset) | 2 byte |
| `0x40` | `0x0801` | reset del chipset | nessuno |

Descrittore DMA (6 byte): opzioni su 3 byte big-endian, poi numero di **parole**
(byte / 2) su 3 byte little-endian. Il canale va in `wValue`.

Note:

- Le scritture spostano sempre almeno 2 byte. Per cambiare un solo registro il
  driver legge prima il registro successivo e lo riscrive invariato
  (`IWrite_Byte` in `hp3900`).
- Il blocco completo dei registri parte da `0xE800`: il driver HP lo legge con
  lunghezza `0x0719` (1817 byte) e lo scrive con 1818 byte.
- Altre zone usate: `0xEF08`-`0xEF09`, `0xF8FF`-`0xF910`, `0xFE11`-`0xFE3E`.
- Il driver HP **scrive l'EEPROM** durante le scansioni (indirizzi `0x78`, `0x79`
  e il contatore delle scansioni in `0x21`-`0x24`). Il nostro driver non scrive mai
  l'EEPROM.

## Pulsanti

Un byte su `0x83`: `0x00` Scan (sinistro), `0x02` Copy (centrale),
`0x03` HP Instant Share (destro). Dopo l'evento il driver HP rilegge lo stato dei
registri, compreso il blocco da `0xE800`.

Senza driver HP i tasti si leggono dal registro `0xE968` (a riposo `0xBF`, un tasto
premuto porta a 0 il suo bit): Scan (sinistro) bit `0x04` (`0xBB`), Copy (centrale)
bit `0x02` (`0xBD`), Instant Share (destro) bit `0x01` (`0xBE`). Il bit 7 vale sempre
1: una lettura `0x00` (capita quando WIA apre la sua app e reimposta il chip) va
scartata.

Windows (WIA) vede anche lui le pressioni e apre l'app «Nuova digitalizzazione». Il
gestore di ogni evento sta in
`HKLM\SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-810F-11D0-BEC7-08002BE2092F}\NNNN\Events\3770_SCAN`
(e `3770_COPY`, `3770_SHARE`), valore `DefaultHandler`:
`{E0372B7D-E115-4525-BC55-B629E68C745A}` è «nessuna azione», la scelta della scheda
Eventi; l'app lo imposta dalle Preferenze.

## Dati immagine

- Lo scanner manda sempre RGB a 8 bit per canale: grigio e bianco/nero
  trasferiscono la stessa quantità di dati del colore, la conversione è dell'host.
- 1200 dpi è la risoluzione ottica; 2400 dpi è interpolata.
- Annulla: il driver HP non ferma lo scanner, scarica tutta la scansione.
- Lampada: circa 50 s di riscaldamento a freddo; si spegne da sola dopo circa
  10 minuti senza traffico USB (timer interno).
- Con un alimentatore troppo debole la lampada si accende ma le immagini sono
  nere e la striscia bianca di calibrazione legge circa 87/255.

## Una scansione, passo per passo

Sequenza del driver HP, riprodotta dal nostro driver e verificata byte per byte sulle
catture 01, 02, 04, 05, 06, 06b, 09 e 10 (test `whole_scan_session_matches_the_hp_driver`).

1. **Preparazione** (`prepare_scan`): stato della lampada, blocco iniziale dei registri,
   timer della lampada spento, canali CCD accesi, attesa del carrello a casa, reset DMA
   con una lettura di 512 byte della RAM del chip che viene scartata, PWM della lampada.
   Il driver HP qui legge e scrive l'EEPROM (contatori): noi no.
2. **Offset** dell'ADC: due letture di una riga (offset 300 e 350, due volte sotto i
   1200 dpi), previsione lineare, poi controlli di 10 righe che alzano di 5 (3 a 1200 dpi)
   i canali ancora sotto il livello 10 dei pixel neri ottici.
3. **Guadagno**: 10 righe sulla striscia bianca, il guadagno porta il decimo più chiaro
   delle colonne all'80 % di 245.
4. **Shading nero**: lampada spenta, 10 righe, un livello per canale (e per parità a 1200 dpi).
5. **Shading bianco**: lampada accesa, 24 righe a 200 dpi con il carrello in movimento e lo
   shading nero attivo, un moltiplicatore per pixel (8192 = 1).
6. **Ritorno a casa** a velocità costante (`[MotorMove2]`: clock 2, CTPC 1991).
7. **Scansione**: tabelle dei passi, gamma identità, shading, avvio, dati a pezzi di
   64512 byte, arresto (con ritorno a casa da 600 dpi in su).

Ogni fase lavora su una propria copia del blocco dei registri; il bit 3 di `E955` (stato
del chip dopo lo spegnimento della lampada) non entra mai nei blocchi.

## Dall'area al chip e dal chip all'immagine

- Origine del vetro (pixel, righe dalla posizione di casa): 200 dpi 43, 69; 300 dpi 65,
  102; 600 dpi 129, 198; 1200 dpi 259, 373; 2400 dpi 259, 746. Bordo sinistro e larghezza
  su multipli di 4.
- 2400 dpi: il chip legge 1200 pixel per pollice e 2400 righe; le righe dei colori (48 di
  distanza) e dei pixel pari/dispari (8) le riallinea l'host, che poi raddoppia la larghezza.
  Il carrello raggiunge l'area alla velocità di scansione, circa 7,5 s/cm.
- Curva tonale dell'host, come il driver HP: `255 × (x / 196)^(1/2.2)` (196 = 245 × 80 %).
- Grigio: media dei tre canali lineari. Bianco e nero: bianco da `255 − 55` in su
  (soglia WIA predefinita 55).

## Catture di riferimento

| File | Contenuto |
| --- | --- |
| `00_collegamento` | collegamento, descrittori, inizializzazione |
| `01_anteprima`, `02` | colore 200 dpi A4 |
| `04` | colore 300 dpi A4 |
| `05` | colore 600 dpi, 5 x 5 cm |
| `06`, `06b` | colore 1200 e 2400 dpi, 2 x 2 cm |
| `07`, `08` | grigio e bianco/nero 300 dpi A4 |
| `09` | colore 300 dpi, area al centro |
| `10` | colore 300 dpi, luminosità e contrasto +500 |
| `11` | scansione annullata al 50 % |
| `12`-`15` | pulsanti |
| `16` | 15 minuti di inattività |

Mancano: diapositiva e negativo con l'adattatore per trasparenti.
