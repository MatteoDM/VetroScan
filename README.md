# scanner — driver aperto per HP Scanjet 3770

Driver userspace e applicazione per lo scanner **HP Scanjet 3770** (USB `03f0:2505`),
scritti interamente in Rust, per Linux, macOS e Windows.

Lo scanner usa un ASIC **Realtek RTS8822**, lo stesso pilotato dal backend SANE
`hp3900`. Il driver è una traduzione in Rust della parte di `hp3900` che serve al
3770, completata con i parametri estratti dal driver HP originale e verificata
sulle catture USB del driver HP.

> Stato: fase iniziale. C'è il nucleo del protocollo USB e il decodificatore delle
> catture; la scansione vera non è ancora implementata.

## Struttura

| Crate | Cosa contiene |
| --- | --- |
| [`hp3770-core`](crates/hp3770-core) | Protocollo USB dell'RTS8822 (richieste vendor, registri, EEPROM, DMA), identità del dispositivo, pulsanti, trait `Transport` indipendente dalla libreria USB |
| [`hp3770-capture`](crates/hp3770-capture) | Lettore di catture USBPcap e strumento `hp3770-decode`, che trasforma una cattura in una sequenza leggibile di operazioni sul chip |

In arrivo: backend USB (`nusb`), driver di scansione, CLI `hp3770`, app desktop (iced),
backend SANE.

## Compilare e provare

Serve Rust 1.80 o più recente.

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

## Decodificare una cattura

```sh
cargo run --release --bin hp3770-decode -- cattura.pcap          # tutte le operazioni
cargo run --release --bin hp3770-decode -- --writes cattura.pcap # solo ciò che modifica lo scanner
cargo run --release --bin hp3770-decode -- --summary *.pcap      # totali per file
```

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
