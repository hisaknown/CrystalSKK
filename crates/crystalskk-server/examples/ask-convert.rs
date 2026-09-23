//! 動いている辞書サーバに、変換の並べ替えを尋ねてみる。
//!
//! `cargo run -p crystalskk-server --example ask-convert -- どうき 犯行の`
//!
//! 同じ読みを、前の文章を添えた変換 (convert) と、添えない検索 (search) で
//! 引き、並びを見比べる。言語モデルで並べ替えているかを確かめるのに使う。

use std::time::Instant;

use crystalskk_core::dict::Query;
use crystalskk_ipc::{Request, Response};
use crystalskk_server::client;

fn show(label: &str, request: Request) {
    let started = Instant::now();
    match client::ask(&request) {
        Ok(Response::Ok(candidates)) => {
            let words: Vec<_> = candidates.iter().take(7).map(|c| c.word.as_str()).collect();
            println!(
                "{label:>8} {:>6.1}ms {}",
                started.elapsed().as_secs_f64() * 1000.0,
                words.join(" ")
            );
        }
        other => println!("{label:>8} {other:?}"),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let key = args.next().expect("読みを渡す");
    let before = args.next().unwrap_or_default();
    let after = args.next().unwrap_or_default();
    let query = Query::okuri_nashi(key);
    show("search", Request::Search(query.clone()));
    show(
        "convert",
        Request::Convert {
            query,
            before,
            after,
        },
    );
}
