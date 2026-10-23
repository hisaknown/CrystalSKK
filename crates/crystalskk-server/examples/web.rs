//! ネットの候補を一つ引いてみる (ADR-0042)。
//!
//! ```text
//! cargo run -p crystalskk-server --example web -- くりすたる
//! cargo run -p crystalskk-server --example web -- おく r り
//! ```

use crystalskk_core::dict::Query;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let query = match args.as_slice() {
        [midashi] => Query::okuri_nashi(midashi.as_str()),
        [midashi, head, okuri] => {
            Query::okuri_ari(midashi, head.chars().next().unwrap_or('a'), okuri.as_str())
        }
        _ => {
            eprintln!("使い方: web <見出し語> [<送り仮名の子音> <送り仮名>]");
            std::process::exit(2);
        }
    };
    match crystalskk_server::web::google(&query) {
        Ok(found) => {
            for candidate in found {
                println!("{}", candidate.word);
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
