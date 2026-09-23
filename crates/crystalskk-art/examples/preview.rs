//! 絵を PNG に描いて見る。
//! `cargo run -p crystalskk-art --example preview -- <出力先> <SVG>...`
fn main() {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().expect("出力先を指定してください"));
    std::fs::create_dir_all(&out).unwrap();
    for svg_path in args {
        let svg = std::fs::read_to_string(&svg_path).unwrap();
        let stem = std::path::Path::new(&svg_path)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        for size in [16, 24, 32, 64] {
            match crystalskk_art::rasterize(&svg, size) {
                Ok(image) => {
                    let png = crystalskk_art::png(&image).unwrap();
                    std::fs::write(out.join(format!("{stem}-{size}.png")), png).unwrap();
                }
                Err(e) => eprintln!("{svg_path}: {e}"),
            }
        }
    }
}
