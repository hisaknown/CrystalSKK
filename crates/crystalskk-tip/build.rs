//! 絵を用意する。
//!
//! 正本は `assets/icons/` の SVG である (ADR-0024)。ここで各大きさに描き、
//! DLL に埋め込む形にする。**描いたものはリポジトリに置かない。** 絵を
//! 直すときは SVG を差し替えるだけで済む。
//!
//! - 入力モードの絵は濃さ (透過度) だけを取り出す。色は動くときに
//!   タスクバーの明るさに合わせて付ける。
//! - 顔の絵は色付きの `.ico` にする。導入のときに書き出し、設定画面の
//!   一覧に出す (ADR-0009)。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// 入力モードの絵を描いておく大きさ。
///
/// トレイの絵は 100% で 16 画素、そこから拡大率に合わせて大きくなる
/// (125% で 20、150% で 24 …)。**近い大きさを縮めて使うと滲む**ので、
/// よく使われる倍率の分はそれぞれ描いておく。
const MODE_SIZES: [u32; 9] = [16, 20, 24, 28, 32, 36, 40, 48, 64];

/// 顔の絵を描いておく大きさ。一覧の小さな絵から、大きく出す場面までを覆う。
const FACE_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// 入力モードの絵。`(Rust での名前, ファイル名)`。
const MODES: [(&str, &str); 6] = [
    ("HIRAGANA", "mode-hiragana"),
    ("KATAKANA", "mode-katakana"),
    ("HALFWIDTH_KATAKANA", "mode-halfwidth-katakana"),
    ("ASCII", "mode-ascii"),
    ("FULLWIDTH_ASCII", "mode-fullwidth-ascii"),
    ("OFF", "mode-off"),
];

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo が渡す"));
    let assets = manifest.join("../../assets/icons");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo が渡す"));
    println!("cargo:rerun-if-changed={}", assets.display());

    let mut generated = String::from("// build.rs が作る。手で直さない。\n\n");

    for (name, file) in MODES {
        let svg = read(&assets, file);
        writeln!(generated, "pub const {name}: Mode = &[").unwrap();
        for size in MODE_SIZES {
            let image = crystalskk_art::rasterize(&svg, size)
                .unwrap_or_else(|e| panic!("{file}.svg を描けません: {e}"));
            let path = out.join(format!("{file}-{size}.bin"));
            std::fs::write(&path, image.coverage()).expect("書き出せる");
            writeln!(generated, "    ({size}, include_bytes!({path:?})),").unwrap();
        }
        generated.push_str("];\n\n");
    }

    let svg = read(&assets, "face");
    let images: Vec<_> = FACE_SIZES
        .iter()
        .map(|size| {
            crystalskk_art::rasterize(&svg, *size)
                .unwrap_or_else(|e| panic!("face.svg を描けません: {e}"))
        })
        .collect();
    let ico = crystalskk_art::ico(&images).expect(".ico にできる");
    let path = out.join("face.ico");
    std::fs::write(&path, ico).expect("書き出せる");
    writeln!(
        generated,
        "pub const FACE_ICO: &[u8] = include_bytes!({path:?});"
    )
    .unwrap();

    std::fs::write(out.join("icons.rs"), generated).expect("書き出せる");
}

fn read(assets: &Path, file: &str) -> String {
    let path = assets.join(format!("{file}.svg"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} を読めません: {e}", path.display()))
}
