// SPDX-License-Identifier: GPL-2.0-or-later
//! Compares an 8-bit and a 16-bit linear scan of the same area
//! (`hp3770 scan --linear` with and without `--depth 16`): levels, noise
//! and the gradation of the dark tones.

fn read(path: &str) -> (usize, usize, Vec<u32>, u32) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let data: Vec<u32> = match info.bit_depth {
        png::BitDepth::Sixteen => buf[..info.buffer_size()]
            .chunks_exact(2)
            .map(|w| u32::from(u16::from_be_bytes([w[0], w[1]])))
            .collect(),
        _ => buf[..info.buffer_size()]
            .iter()
            .map(|&v| u32::from(v) * 257)
            .collect(),
    };
    let bits = if info.bit_depth == png::BitDepth::Sixteen {
        16
    } else {
        8
    };
    (info.width as usize, info.height as usize, data, bits)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (w, h, a, _) = read(&args[1]);
    let (_, _, b, _) = read(&args[2]);
    for (name, img) in [("8 bit ", &a), ("16 bit", &b)] {
        // Flat area: the lid, top-left 200 x 200, green channel.
        let patch: Vec<f64> = (0..200)
            .flat_map(|y| (0..200).map(move |x| (y * w + x) * 3 + 1))
            .map(|i| f64::from(img[i]))
            .collect();
        let mean = patch.iter().sum::<f64>() / patch.len() as f64;
        let sd =
            (patch.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / patch.len() as f64).sqrt();
        // Dark tones: samples below 4 % of full scale, counted in steps
        // of 1/4096 of full scale.
        let dark: std::collections::BTreeSet<u32> =
            img.iter().filter(|&&v| v < 2600).map(|&v| v / 16).collect();
        let darks = img.iter().filter(|&&v| v < 2600).count();
        println!(
            "{name}: lid green mean {:.1} sd {:.1} (in 8-bit levels: {:.2} ± {:.2}); \
             {darks} dark samples in {} distinct levels of 1/4096",
            mean,
            sd,
            mean / 257.0,
            sd / 257.0,
            dark.len()
        );
    }
    let _ = h;
}
