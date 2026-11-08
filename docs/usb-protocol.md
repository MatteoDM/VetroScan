# USB protocol of the HP Scanjet 3770

A summary of what is confirmed by USB captures of the original HP driver (Windows 11, HP
driver 8.0.0.0, captures of 7 November 2026) and by the SANE `hp3900` backend
(`hp3900_usb.c`, `hp3900_rts8822.c`).

## Device

| Item | Value |
| --- | --- |
| Vendor / Product | `03f0` / `2505` |
| ASIC | Realtek RTS8822 (exact marking to be read on the chip) |
| Interface | 1 interface, vendor class (`FF`), 3 endpoints |
| Endpoint `0x81` | bulk IN: image and calibration data |
| Endpoint `0x02` | bulk OUT: tables (gamma, shading) |
| Endpoint `0x83` | interrupt IN: buttons, 1 byte |
| Firmware | inside the ASIC, nothing to load |

## Control requests

Every operation on the chip is a vendor request on EP0 with `bRequest = 0x04`. `wValue` is
the address, `wIndex` picks the operation.

| `bmRequestType` | `wIndex` | Operation | Data |
| --- | --- | --- | --- |
| `0xC0` | `0x0100` | read registers from `wValue` | `wLength` bytes returned |
| `0x40` | `0x0000` | write registers from `wValue` | the bytes to write |
| `0xC0` / `0x40` | `0x0200` | read / write the EEPROM | 16-bit words |
| `0x40` | `0x0400` | enable DMA for reading (to the host) | 6 bytes |
| `0x40` | `0x0401` | enable DMA for writing (to the scanner) | 6 bytes |
| `0x40` | `0x0600` | cancel DMA | 2 zero bytes |
| `0x40` | `0x0800` | DMA control word (`0` = reset) | 2 bytes |
| `0x40` | `0x0801` | chipset reset | none |

DMA descriptor (6 bytes): options on 3 bytes big-endian, then the number of **words**
(bytes / 2) on 3 bytes little-endian. The channel goes in `wValue`.

Notes:

- Writes always move at least 2 bytes. To change a single register the driver first reads
  the next register and writes it back unchanged (`IWrite_Byte` in `hp3900`).
- The full register block starts at `0xE800`: the HP driver reads it with length `0x0719`
  (1817 bytes) and writes it with 1818 bytes.
- Other areas used: `0xEF08`-`0xEF09`, `0xF8FF`-`0xF910`, `0xFE11`-`0xFE3E`.
- The HP driver **writes the EEPROM** during scans (addresses `0x78`, `0x79` and the scan
  counter at `0x21`-`0x24`). Our driver never writes the EEPROM.

## Buttons

One byte on `0x83`: `0x00` Scan (left), `0x02` Copy (centre), `0x03` HP Instant Share
(right). After the event the HP driver reads the registers again, the block from `0xE800`
included.

Without the HP driver the buttons are read from register `0xE968` (`0xBF` at rest, a
pressed button clears its bit): Scan (left) bit `0x04` (`0xBB`), Copy (centre) bit `0x02`
(`0xBD`), Instant Share (right) bit `0x01` (`0xBE`). Bit 7 is always 1: a read of `0x00`
(it happens when WIA opens its app and resets the chip) is to be discarded.

Windows (WIA) also sees the presses and opens its "New Scan" app. The handler of each event
is in
`HKLM\SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-810F-11D0-BEC7-08002BE2092F}\NNNN\Events\3770_SCAN`
(and `3770_COPY`, `3770_SHARE`), value `DefaultHandler`:
`{E0372B7D-E115-4525-BC55-B629E68C745A}` is "take no action", the choice of the Events tab;
the app sets it from its Preferences.

## Image data

- The scanner always sends RGB at 8 bits per channel: grey and black/white transfer as much
  data as colour, the conversion is done on the host.
- 1200 dpi is the optical resolution; 2400 dpi is interpolated.
- Cancel: the HP driver does not stop the scanner, it downloads the whole scan.
- Lamp: about 50 s of warm-up from cold; it switches itself off after about 10 minutes
  without USB traffic (internal timer).
- With a power supply that is too weak the lamp switches on but the images are black and
  the white calibration strip reads about 87/255.

## A scan, step by step

The HP driver's sequence, reproduced by our driver and checked byte for byte against
captures 01, 02, 04, 05, 06, 06b, 09 and 10 (test `whole_scan_session_matches_the_hp_driver`).

1. **Preparation** (`prepare_scan`): lamp state, initial register block, lamp timer off,
   CCD channels on, wait for the carriage at home, DMA reset with a 512-byte read of the
   chip's RAM that is thrown away, lamp PWM. The HP driver reads and writes the EEPROM
   (counters) here: we do not.
2. ADC **offset**: two reads of one line (offsets 300 and 350, twice below 1200 dpi), a
   linear prediction, then checks of 10 lines that raise by 5 (3 at 1200 dpi) the channels
   still below level 10 of the optical black pixels.
3. **Gain**: 10 lines on the white strip, the gain brings the brightest tenth of the columns
   to 80 % of 245.
4. **Black shading**: lamp off, 10 lines, one level per channel (and per parity at 1200 dpi).
5. **White shading**: lamp on, 24 lines at 200 dpi with the carriage moving and black
   shading active, one multiplier per pixel (8192 = 1).
6. **Back home** at constant speed (`[MotorMove2]`: clock 2, CTPC 1991).
7. **Scan**: step tables, identity gamma, shading, start, data in pieces of 64512 bytes,
   stop (with the carriage back home from 600 dpi up).

Each phase works on its own copy of the register block; bit 3 of `E955` (the chip's state
after the lamp went off) never goes into the blocks.

## From the area to the chip and from the chip to the image

- Origin of the glass (pixels, lines from the home position): 200 dpi 43, 69; 300 dpi 65,
  102; 600 dpi 129, 198; 1200 dpi 259, 373; 2400 dpi 259, 746. Left edge and width on
  multiples of 4.
- 2400 dpi: the chip reads 1200 pixels per inch and 2400 lines; the host realigns the
  colour lines (48 apart) and the even/odd pixels (8), then doubles the width. The carriage
  reaches the area at scanning speed, about 7.5 s/cm.
- Host tone curve, like the HP driver: `255 × (x / 196)^(1/2.2)` (196 = 245 × 80 %).
- Grey: the mean of the three linear channels. Black and white: white from `255 − 55` up
  (default WIA threshold 55).

## Reference captures

| File | Content |
| --- | --- |
| `00_collegamento` | connection, descriptors, initialisation |
| `01_anteprima`, `02` | colour 200 dpi A4 |
| `04` | colour 300 dpi A4 |
| `05` | colour 600 dpi, 5 x 5 cm |
| `06`, `06b` | colour 1200 and 2400 dpi, 2 x 2 cm |
| `07`, `08` | grey and black/white 300 dpi A4 |
| `09` | colour 300 dpi, area in the middle |
| `10` | colour 300 dpi, brightness and contrast +500 |
| `11` | scan cancelled at 50 % |
| `12`-`15` | buttons |
| `16` | 15 minutes idle |

Missing: slide and negative with the transparency adapter.

(The capture files keep their Italian names: the tests refer to them.)
