// SPDX-License-Identifier: GPL-2.0-or-later
//! Turns a linear scan saved as PNG (`hp3770 scan --linear`) into the RAW
//! files of the application, to try them in raw editors:
//! `cargo run --example raw_from_png -- scan.png out` writes out.dng and
//! out.tif.

use hp3770_core::geometry::HP_WHITE;
use hp3770_core::session::{Format, Image};

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
    let dpi = reader
        .info()
        .pixel_dims
        .map_or(300, |d| (f64::from(d.xppu) * 0.0254).round() as u16);
    let image = Image {
        width: info.width as usize,
        height: info.height as usize,
        dpi,
        format: Format::Rgb,
        data,
    };
    let raw = hp3770_export::Linear16::from_linear8(&image, HP_WHITE)?;
    hp3770_export::save_dng(format!("{output}.dng").as_ref(), &raw)?;
    hp3770_export::save_tiff16(format!("{output}.tif").as_ref(), &raw)?;
    println!("scritti {output}.dng e {output}.tif ({} dpi)", dpi);
    Ok(())
}
