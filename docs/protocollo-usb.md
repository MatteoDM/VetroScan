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
- Il driver HP **scrive l'EEPROM** durante le scansioni (es. indirizzi `0x78`,
  `0x79`), probabilmente contatori. Il nostro driver non deve scrivere l'EEPROM
  finché non sappiamo cosa contiene.

## Pulsanti

Un byte su `0x83`: `0x00` Scan (sinistro), `0x02` Copy (centrale),
`0x03` HP Instant Share (destro). Dopo l'evento il driver HP rilegge lo stato dei
registri, compreso il blocco da `0xE800`.

## Dati immagine

- Lo scanner manda sempre RGB a 8 bit per canale: grigio e bianco/nero
  trasferiscono la stessa quantità di dati del colore, la conversione è dell'host.
- 1200 dpi è la risoluzione ottica; 2400 dpi è interpolata.
- Annulla: il driver HP non ferma lo scanner, scarica tutta la scansione.
- Lampada: circa 50 s di riscaldamento a freddo; si spegne da sola dopo circa
  10 minuti senza traffico USB (timer interno).
- Con un alimentatore troppo debole la lampada si accende ma le immagini sono
  nere e la striscia bianca di calibrazione legge circa 87/255.

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
