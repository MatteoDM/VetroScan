# Epson 4490 glass captures, stripped

What Epson's drivers wrote to the Epson Perfection 4490 and what it answered, in 31 USB
captures of glass scans made on 11-14 November 2026 (`captures/epson4490/*.pcap`, outside the
repository; WIA but `13`, `14`, `15`, `38`-`43` and `57`-`62`, Epson Scan: at 16 bits, `38`-`43`
the whole glass at 100-1200 dpi, `57` and `58` at 2400 dpi, `14` and `59`-`62` at 4800 dpi
(`61` at the glass's left edge), 16 and 8 bits, their image data left out). The tests of
`setup.rs` rebuild every set-up block of each one from its request, and those of `scan.rs` run
the whole native scan against eleven of the WIA ones, write by write.

Made from the captures with `tools/strip.py` (`captures/epson4490/tools`, on `stream.py`;
`--no-data` for those whose image data is left out), one step per line:

| line | what |
|---|---|
| `W HEX` | the driver wrote these bytes |
| `R HEX` | the scanner answered these bytes |
| `A HEX` | an answer that changes from one session to another (`1E 53`, `1E 86`, `1E 87`, `1E 92`) |
| `P CMD HEX` | the driver polled `CMD` (`1B 03`, `1E 04`); `HEX` is the last answer |
| `D N` | N bytes of image data (left out); the block's trailer follows as `R` |
| `W =gamma` | WIA's gamma table after `1E B3` (`tables::gamma`) |
| `W =tone` | WIA's tone table (`1E 84` type 02, `tables/wia_tone.bin`) |
| `W =shading N` | the shading table, N bytes (computed from the calibration) |
| `W =42 HEX` | a `1E 42` block (computed from the calibration) |
| `W =other N`, `R =other N` | a long table left out (Epson Scan's tables, capture 05's `1E 83`) |

Windows' button polls (`1E 85`) are left out.
