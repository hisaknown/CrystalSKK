//! 辞書を読み込み、所要時間と検索の速さを測る。
//!
//! ```text
//! cargo run --release -p crystalskk-dict --example load-dict -- <辞書>
//! ```

use std::time::Instant;

use crystalskk_core::dict::{CandidateSource, Query};
use crystalskk_dict::{MemoryDict, encoding};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("使い方: load-dict <辞書>");
        std::process::exit(2);
    };

    let started = Instant::now();
    let bytes = std::fs::read(&path)?;
    let read = started.elapsed();

    let started = Instant::now();
    let decoded = encoding::decode(&bytes);
    let decode = started.elapsed();

    let started = Instant::now();
    let (dict, report) = MemoryDict::parse(&decoded.text);
    let parse = started.elapsed();

    println!("符号化:   {}", decoded.encoding);
    println!(
        "見出し:   {} 件 (読み飛ばし {} 行)",
        report.entries, report.skipped
    );
    println!("読み取り: {:.0} ms", read.as_secs_f64() * 1000.0);
    println!("復号:     {:.0} ms", decode.as_secs_f64() * 1000.0);
    println!("解析:     {:.0} ms", parse.as_secs_f64() * 1000.0);

    // 検索を繰り返して一件あたりの時間を見る (PRD N-02: p99 < 5ms)。
    let queries: Vec<Query> = ["かんじ", "にほんご", "へんかん", "しらないことば", "あ"]
        .iter()
        .map(|k| Query::okuri_nashi(*k))
        .collect();
    let rounds = 20_000;
    let started = Instant::now();
    let mut found = 0usize;
    for _ in 0..rounds {
        for query in &queries {
            found += dict.lookup(query).len();
        }
    }
    let per_lookup = started.elapsed().as_secs_f64() / (rounds * queries.len()) as f64;
    println!(
        "検索:     {:.1} µs/件 (候補 {found} 件)",
        per_lookup * 1_000_000.0
    );

    let started = Instant::now();
    let completions = dict.complete("かん", 10).len();
    println!(
        "補完:     {:.1} µs ({completions} 件)",
        started.elapsed().as_secs_f64() * 1_000_000.0
    );
    Ok(())
}
