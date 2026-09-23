//! 本物の言語モデルで並べる試験。
//!
//! モデルも llama.cpp もリポジトリには無いので、次の環境変数で場所が
//! 与えられたときだけ走る。与えられなければ何もせずに通る。
//!
//! - `CRYSTALSKK_LM_RUNTIME`: llama.dll のあるフォルダ
//! - `CRYSTALSKK_LM_MODEL`: GGUF
//! - `CRYSTALSKK_LM_TOKENIZER`: tokenizer.json

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crystalskk_core::dict::{Candidate, Context, Query, Ranker};
use crystalskk_lm::{LlamaScorer, LmRanker, Policy};

fn ranker() -> Option<LmRanker<LlamaScorer>> {
    let path = |name| std::env::var_os(name).map(PathBuf::from);
    let (runtime, model, tokenizer) = (
        path("CRYSTALSKK_LM_RUNTIME")?,
        path("CRYSTALSKK_LM_MODEL")?,
        path("CRYSTALSKK_LM_TOKENIZER")?,
    );
    let scorer = LlamaScorer::load(&runtime, &model, &tokenizer, 4).expect("読み込める");
    Some(LmRanker::new(
        scorer,
        Policy {
            weight: 1.0,
            deadline: Duration::from_secs(5),
            before: 100,
            after: 5,
        },
    ))
}

fn rank(
    ranker: &LmRanker<LlamaScorer>,
    key: &str,
    words: &[&str],
    before: &str,
    after: &str,
) -> Vec<String> {
    let mut candidates: Vec<Candidate> = words.iter().map(|w| Candidate::new(*w)).collect();
    let context = Context {
        preceding_text: Some(before.to_owned()),
        following_text: Some(after.to_owned()),
        ..Context::default()
    };
    let started = Instant::now();
    ranker.rank(&context, &Query::okuri_nashi(key), &mut candidates);
    eprintln!("{key}: {:?}", started.elapsed());
    candidates.into_iter().map(|c| c.word).collect()
}

#[test]
fn the_context_decides_between_homophones() {
    let Some(ranker) = ranker() else {
        eprintln!("言語モデルの場所が与えられていないので飛ばす");
        return;
    };
    let words = ["漢字", "感じ", "幹事", "監事"];
    assert_eq!(
        rank(&ranker, "かんじ", &words, "忘年会の", "を引き受けた")[0],
        "幹事"
    );
    assert_eq!(
        rank(&ranker, "かんじ", &words, "小学校で習う", "の書き取り")[0],
        "漢字"
    );
    let words = ["同期", "動悸", "動機", "銅器"];
    assert_eq!(
        rank(&ranker, "どうき", &words, "犯行の", "を調べる")[0],
        "動機"
    );
}
