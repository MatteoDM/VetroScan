// SPDX-License-Identifier: GPL-2.0-or-later
//! Turns a linear scan saved as PNG into the RAW files of the application,
//! to try them in raw editors: `cargo run --example raw_from_png -- scan.png
//! out` writes out.dng and out.tif. The PNG is either 8-bit
//! (`hp3770 scan --linear`) or the chip's 16-bit samples
//! (`hp3770 scan --linear --depth 16`, the file NAME-16bit.png).

use hp3770_core::geometry::HP_WHITE;
use scan_imaging::{Format, Image};

fn main() -> hp3770_export::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output] = args.as_slice() else {
        return Err("uso: raw_from_png scansione.png uscita".into());
    };
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(input)?));
    let mut reader = decoder.read_info()?;
    let mut data = vec![0; reader.output_buffer_size().ok_or("troppo grande")?];
    let info = reader.next_frame(&mut data)?;
    data.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgb {
        return Err("serve un PNG a colori".into());
    }
    let dpi = reader
        .info()
        .pixel_dims
        .map_or(300, |d| (f64::from(d.xppu) * 0.0254).round() as u16);
    let (width, height) = (info.width as usize, info.height as usize);
    let raw = if info.bit_depth == png::BitDepth::Sixteen {
        let samples: Vec<u16> = data
            .chunks_exact(2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .collect();
        hp3770_export::Linear16::from_linear16(
            width,
            height,
            dpi,
            &samples,
            HP_WHITE.map(|w| w * 256.0),
        )
    } else {
        let image = Image {
            width,
            height,
            dpi,
            format: Format::Rgb,
            data,
        };
        hp3770_export::Linear16::from_linear8(&image, HP_WHITE)?
    };
    hp3770_export::save_dng(format!("{output}.dng").as_ref(), &raw)?;
    hp3770_export::save_tiff16(format!("{output}.tif").as_ref(), &raw)?;
    println!("scritti {output}.dng e {output}.tif ({dpi} dpi)");
    Ok(())
}
