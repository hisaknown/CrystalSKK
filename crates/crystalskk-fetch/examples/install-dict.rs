//! 辞書を取得して UTF-8 で保存する。
//!
//! ```text
//! cargo run -p crystalskk-fetch --example install-dict -- <保存先> [URL]
//! ```

use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next().map(PathBuf::from) else {
        eprintln!("使い方: install-dict <保存先> [URL]");
        std::process::exit(2);
    };
    let url = args
        .next()
        .unwrap_or_else(|| crystalskk_fetch::SKK_JISYO_L.to_owned());

    println!("取得元: {url}");
    let started = Instant::now();
    let Some(report) = crystalskk_fetch::install(&url, &path, None)? else {
        println!("変化なし");
        return Ok(());
    };
    let elapsed = started.elapsed();

    println!("保存先:     {}", path.display());
    println!("取得元符号: {}", report.source_encoding);
    println!("見出し:     {} 件", report.entries);
    println!("読み飛ばし: {} 行", report.skipped);
    println!("併合:       {} 行", report.merged);
    println!(
        "保存量:     {:.2} MiB",
        report.bytes_written as f64 / 1_048_576.0
    );
    if let Some(etag) = &report.etag {
        println!("ETag:       {etag}");
    }
    println!("所要:       {:.2} 秒", elapsed.as_secs_f64());
    Ok(())
}
