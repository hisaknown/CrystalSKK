//! SVG を `.ico` にする。インストーラがアプリ一覧に出す絵に使う。
//! `cargo run -p crystalskk-art --example ico -- <出力.ico> <SVG>`
use crystalskk_art::{ICO_SIZES, ico, rasterize};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("出力先を指定してください");
    let svg_path = args.next().expect("SVG を指定してください");
    let svg = std::fs::read_to_string(&svg_path).expect("SVG を読める");
    let images: Vec<_> = ICO_SIZES
        .iter()
        .map(|size| rasterize(&svg, *size).expect("描ける"))
        .collect();
    std::fs::write(&out, ico(&images).expect(".ico にできる")).expect("書き出せる");
}
