//! 明るい地と暗い地に並べた見本を一枚にする。
//! `cargo run -p crystalskk-art --example sheet -- <出力.png> <SVG>...`
//!
//! 名前が `mode-` で始まる絵は濃さだけを使い、地に合わせて黒か白で塗る。
//! それ以外 (顔) は色のまま置く。
use crystalskk_art::{Image, png, rasterize};

const SIZES: [u32; 4] = [16, 24, 32, 64];
const GAP: u32 = 8;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("出力先を指定してください");
    let svgs: Vec<(String, String)> = args
        .map(|path| {
            let stem = std::path::Path::new(&path)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (stem, std::fs::read_to_string(&path).unwrap())
        })
        .collect();

    let cell = SIZES.iter().sum::<u32>() + GAP * (SIZES.len() as u32 + 1);
    let row = 64 + GAP * 2;
    let width = cell;
    let height = row * svgs.len() as u32;
    let sheet_width = width * 2;
    let mut sheet = vec![0u8; (sheet_width * height * 4) as usize];

    for (half, (bg, ink)) in [
        ([0xF3u8, 0xF3, 0xF3], [0u8, 0, 0]),
        ([0x20, 0x20, 0x20], [0xFF, 0xFF, 0xFF]),
    ]
    .iter()
    .enumerate()
    {
        for y in 0..height {
            for x in 0..width {
                let at = ((y * sheet_width + x + half as u32 * width) * 4) as usize;
                sheet[at..at + 4].copy_from_slice(&[bg[0], bg[1], bg[2], 255]);
            }
        }
        for (index, (stem, svg)) in svgs.iter().enumerate() {
            let mut x0 = half as u32 * width + GAP;
            for size in SIZES {
                let image: Image = rasterize(svg, size).unwrap();
                let y0 = index as u32 * row + GAP + (64 - size) / 2;
                for y in 0..size {
                    for x in 0..size {
                        let src = ((y * size + x) * 4) as usize;
                        let [r, g, b, a] = if stem.starts_with("mode-") {
                            [ink[0], ink[1], ink[2], image.rgba[src + 3]]
                        } else {
                            [
                                image.rgba[src],
                                image.rgba[src + 1],
                                image.rgba[src + 2],
                                image.rgba[src + 3],
                            ]
                        };
                        let at = (((y0 + y) * sheet_width + x0 + x) * 4) as usize;
                        let alpha = u32::from(a);
                        for (channel, value) in [r, g, b].iter().enumerate() {
                            let under = u32::from(sheet[at + channel]);
                            sheet[at + channel] =
                                ((u32::from(*value) * alpha + under * (255 - alpha)) / 255) as u8;
                        }
                    }
                }
                x0 += size + GAP;
            }
        }
    }

    let bytes = encode(&sheet, sheet_width, height);
    std::fs::write(out, bytes).unwrap();
}

/// 横長の RGBA を PNG にする。`png()` は正方形しか扱わないので、
/// 正方形に広げて (余白は透明) 書く。
fn encode(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let side = width.max(height);
    let mut square = vec![0u8; (side * side * 4) as usize];
    for y in 0..height {
        let from = (y * width * 4) as usize;
        let to = (y * side * 4) as usize;
        square[to..to + (width * 4) as usize]
            .copy_from_slice(&rgba[from..from + (width * 4) as usize]);
    }
    png(&Image {
        size: side,
        rgba: square,
    })
    .unwrap()
}
