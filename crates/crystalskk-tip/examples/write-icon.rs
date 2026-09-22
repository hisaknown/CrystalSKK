//! モードの絵を `.ico` として書き出す。
//!
//! ```text
//! cargo run -p crystalskk-tip --example write-icon -- <書き出す先> [文字] [大きさ]
//! ```
//!
//! 絵柄を見て確かめるためのもの。導入のときに書き出されるものと同じ。

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("使い方: write-icon <書き出す先> [文字] [大きさ]");
        std::process::exit(2);
    };
    let label = args.next().unwrap_or_else(|| "あ".to_owned());
    let size: i32 = args.next().as_deref().unwrap_or("32").parse()?;

    let bytes = crystalskk_tip::icon::ico_bytes(&label, size).ok_or("絵を作れませんでした")?;
    std::fs::write(&path, &bytes)?;

    println!(
        "{} に書きました ({} バイト, {size} 画素, 「{label}」)",
        path,
        bytes.len()
    );
    Ok(())
}
