//! 試験で使う設定。
//!
//! エンジンは既定値を持たない (ADR-0020)。試験でも値は**ここで明示して**
//! 渡す。値そのものは同梱の雛形と揃えてあるが、揃っている必要はない。
//! 試験が確かめるのは「渡した値のとおりに振る舞うか」である。

#![allow(dead_code)]

use crystalskk_core::dict::CandidateSource;
use crystalskk_core::options::{CandidateOptions, CompletionOptions, Options};
use crystalskk_core::{Engine, RomajiTable};

/// 一覧から候補を選ぶキー。
pub const LABELS: [char; 7] = ['a', 's', 'd', 'f', 'j', 'k', 'l'];

/// 一ページの候補の数。
pub const PAGE_SIZE: usize = LABELS.len();

/// 何回目の変換から一覧に移るか。
pub const UNTIL_LIST: usize = 5;

/// 補完候補を受け取るキー。
pub const TAKE_KEY: char = '.';

pub fn options() -> Options {
    Options {
        completion: CompletionOptions {
            dynamic: true,
            min_length: 2,
            limit: 16,
            take_key: TAKE_KEY,
        },
        candidates: CandidateOptions {
            until_list: UNTIL_LIST,
            labels: LABELS.to_vec(),
        },
        romaji: RomajiTable::parse(ROMAJI).expect("雛形は読める"),
    }
}

/// 同梱のローマ字テーブル。試験でも規則はファイルから読む。
const ROMAJI: &str = include_str!("../../../crystalskk-settings/romaji.txt");

/// 設定を渡したエンジン。
pub fn engine(dict: Box<dyn CandidateSource>) -> Engine {
    let mut engine = Engine::new(dict);
    engine.configure(options());
    engine
}
