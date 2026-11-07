# scanner — driver aperto per HP Scanjet 3770

Driver userspace e applicazione per lo scanner **HP Scanjet 3770** (USB `03f0:2505`),
scritti interamente in Rust, per Linux, macOS e Windows.

Lo scanner usa un ASIC **Realtek RTS8822**, lo stesso pilotato dal backend SANE
`hp3900`. Il driver è una traduzione in Rust della parte di `hp3900` che serve al
3770, completata con i parametri estratti dal driver HP originale e verificata
sulle catture USB del driver HP.

> Stato: fase iniziale. Ci sono il nucleo del protocollo USB, il decodificatore delle
> catture, l'accesso USB allo scanner e un comando di sola lettura per i primi test
> sull'hardware; la scansione vera non è ancora implementata.

## Struttura

| Crate | Cosa contiene |
| --- | --- |
| [`hp3770-core`](crates/hp3770-core) | Protocollo USB dell'RTS8822 (richieste vendor, registri, EEPROM, DMA), identità del dispositivo, pulsanti, trait `Transport` indipendente dalla libreria USB |
| [`hp3770-capture`](crates/hp3770-capture) | Lettore di catture USBPcap e strumento `hp3770-decode`, che trasforma una cattura in una sequenza leggibile di operazioni sul chip |
| [`hp3770-usb`](crates/hp3770-usb) | Accesso USB: `nusb` (Linux, macOS, Windows con WinUSB) e `usbscan.sys` (Windows con il driver HP installato, nessun cambio di driver) |
| [`hp3770-cli`](crates/hp3770-cli) | Comando `hp3770` |

In arrivo: driver di scansione, app desktop (iced), backend SANE.

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
cargo run --release --bin hp3770 -- probe -o probe.txt       # registri ed EEPROM, sola lettura
cargo run --release --bin hp3770 -- buttons                  # stampa i pulsanti premuti
```

`probe` passa da un filtro che rifiuta ogni scrittura prima che arrivi allo scanner:
non accende la lampada, non muove il carrello e non tocca l'EEPROM. Con `--backend nusb`
o `--backend usbscan` si sceglie il backend; di base su Windows si prova prima
`usbscan.sys`.

Su Linux l'utente deve poter aprire il dispositivo, per esempio con una regola udev:

```text
SUBSYSTEM=="usb", ATTR{idVendor}=="03f0", ATTR{idProduct}=="2505", MODE="0660", TAG+="uaccess"
```

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
