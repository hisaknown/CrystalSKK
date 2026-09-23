//! 変換の候補を、小さな言語モデルで前後の文章から並べる (ADR-0030)。
//!
//! # 採点は生成ではない
//!
//! 「前の文章 + 候補 + 後ろの文章」を候補ごとにトークンに分け、その同時確率を
//! 比べる。全候補で先頭から一致するトークンは一度だけ計算して共有し、残りを
//! 一つのバッチで流す。前の文章の末尾と候補が一つのトークンにくっついても、
//! そのトークンは共有から外れて採点に入るので、**トークンの切れ目と候補の
//! 切れ目は揃っていなくてよい。**
//!
//! # 辞書の順も見る
//!
//! 採点だけで並べると、辞書が元から当てていた候補を崩す。辞書の順は長年の
//! 編集で整えられた頻度の順でもあるので、採点に `−weight·ln(順位)` を足して
//! 並べる。
//!
//! # 締め切りを守る
//!
//! 間に合わなければ並びを変えない。言語モデルが無い、読めない、壊れて
//! いるときも同じで、入力は止めない (PRD §3)。

#![deny(unsafe_code)]

mod llama;

use std::cell::RefCell;
use std::path::Path;
use std::time::{Duration, Instant};

use crystalskk_core::dict::{Candidate, Context, Query, Ranker};
use tokenizers::Tokenizer;

pub use llama::VERSION as LLAMA_VERSION;

/// 前の文章として言語モデルに見せるトークンの上限。
const MAX_PREFIX_TOKENS: usize = 384;

/// 候補を採点するもの。
pub trait Scorer {
    /// 各 `texts` を `before` と `after` のあいだに置いたときの、文としての
    /// 自然さ (対数確率)。大きいほど自然。締め切りに間に合わなければ `None`。
    fn score(
        &mut self,
        before: &str,
        texts: &[String],
        after: &str,
        deadline: Instant,
    ) -> Option<Vec<f32>>;
}

/// llama.cpp で動く言語モデル。
#[derive(Debug)]
pub struct LlamaScorer {
    model: llama::Model,
    tokenizer: Tokenizer,
}

impl LlamaScorer {
    /// `runtime` は llama.dll のあるフォルダ、`model` は GGUF、`tokenizer` は
    /// その語彙 (tokenizer.json)。
    pub fn load(
        runtime: &Path,
        model: &Path,
        tokenizer: &Path,
        threads: usize,
    ) -> Result<Self, String> {
        let tokenizer = Tokenizer::from_file(tokenizer)
            .map_err(|e| format!("{} を語彙として読めません: {e}", tokenizer.display()))?;
        let model = llama::Model::load(runtime, model, threads)?;
        Ok(Self { model, tokenizer })
    }
}

impl Scorer for LlamaScorer {
    fn score(
        &mut self,
        before: &str,
        texts: &[String],
        after: &str,
        deadline: Instant,
    ) -> Option<Vec<f32>> {
        let sequences = texts
            .iter()
            .map(|text| {
                let encoding = self
                    .tokenizer
                    .encode(format!("{before}{text}{after}"), true)
                    .ok()?;
                Some(
                    encoding
                        .get_ids()
                        .iter()
                        .map(|&id| id as i32)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Option<Vec<_>>>()?;
        let (prefix, suffixes) = split(&sequences)?;
        // 前の文章が長すぎれば、古いほうを落とす。先頭の特殊トークンは残す。
        let prefix = if prefix.len() > MAX_PREFIX_TOKENS {
            let mut kept = vec![prefix[0]];
            kept.extend_from_slice(&prefix[prefix.len() - (MAX_PREFIX_TOKENS - 1)..]);
            kept
        } else {
            prefix.to_vec()
        };
        self.model.score(&prefix, &suffixes, deadline)
    }
}

/// 全候補に共通する先頭と、候補ごとの残りに分ける。残りは空にしない。
fn split(sequences: &[Vec<i32>]) -> Option<(&[i32], Vec<Vec<i32>>)> {
    let first = sequences.first()?;
    let shortest = sequences.iter().map(Vec::len).min()?;
    let mut shared = (0..shortest)
        .find(|&i| sequences.iter().any(|s| s[i] != first[i]))
        .unwrap_or(shortest);
    // 残りが空の候補があると採点できない。少なくとも一つは残す。
    shared = shared.min(shortest.saturating_sub(1));
    if shared == 0 {
        return None;
    }
    Some((
        &first[..shared],
        sequences.iter().map(|s| s[shared..].to_vec()).collect(),
    ))
}

/// 並べ方の決まり。
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    /// 辞書の順の重み。
    pub weight: f32,
    /// 並べ替えを待つ時間。
    pub deadline: Duration,
    /// 前の文章を何文字見せるか。
    pub before: usize,
    /// 後ろの文章を何文字見せるか。
    pub after: usize,
}

/// 言語モデルで候補を並べるランカー。
#[derive(Debug)]
pub struct LmRanker<S> {
    scorer: RefCell<S>,
    policy: Policy,
}

impl<S: Scorer> LmRanker<S> {
    pub fn new(scorer: S, policy: Policy) -> Self {
        Self {
            scorer: RefCell::new(scorer),
            policy,
        }
    }
}

impl<S: Scorer> Ranker for LmRanker<S> {
    fn rank(&self, context: &Context, query: &Query, candidates: &mut Vec<Candidate>) {
        if candidates.len() < 2 {
            return;
        }
        let before = context.text_before(self.policy.before);
        let after = context.text_after(self.policy.after);
        // 手がかりが何も無ければ、辞書の順より良くなる見込みは薄い。
        if before.is_empty() && after.is_empty() {
            return;
        }
        let texts: Vec<String> = candidates
            .iter()
            .map(|c| c.to_text(query.okuri.as_deref()))
            .collect();
        let deadline = Instant::now() + self.policy.deadline;
        let Some(scores) = self
            .scorer
            .borrow_mut()
            .score(&before, &texts, &after, deadline)
        else {
            return;
        };
        if scores.len() != candidates.len() || Instant::now() > deadline {
            return;
        }
        order(candidates, &scores, self.policy.weight);
    }
}

/// 採点と辞書の順を合わせて並べる。同点なら辞書の順。
fn order(candidates: &mut Vec<Candidate>, scores: &[f32], weight: f32) {
    let mut keyed: Vec<(f32, Candidate)> = candidates
        .drain(..)
        .zip(scores)
        .enumerate()
        .map(|(i, (c, &s))| (s - weight * ((i + 1) as f32).ln(), c))
        .collect();
    keyed.sort_by(|a, b| b.0.total_cmp(&a.0));
    candidates.extend(keyed.into_iter().map(|(_, c)| c));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 決まった点を返す採点係。見せられた文章を覚えておく。
    struct Fixed {
        scores: Vec<f32>,
        seen: Vec<(String, Vec<String>, String)>,
        late: bool,
    }

    impl Scorer for Fixed {
        fn score(
            &mut self,
            before: &str,
            texts: &[String],
            after: &str,
            _: Instant,
        ) -> Option<Vec<f32>> {
            self.seen
                .push((before.to_owned(), texts.to_vec(), after.to_owned()));
            (!self.late).then(|| self.scores.clone())
        }
    }

    fn ranker(scores: &[f32], weight: f32) -> LmRanker<Fixed> {
        LmRanker::new(
            Fixed {
                scores: scores.to_vec(),
                seen: Vec::new(),
                late: false,
            },
            Policy {
                weight,
                deadline: Duration::from_secs(1),
                before: 3,
                after: 2,
            },
        )
    }

    fn words(candidates: &[Candidate]) -> Vec<&str> {
        candidates.iter().map(|c| c.word.as_str()).collect()
    }

    fn context(before: &str, after: &str) -> Context {
        Context {
            preceding_text: Some(before.to_owned()),
            following_text: Some(after.to_owned()),
            ..Context::default()
        }
    }

    fn kanji() -> Vec<Candidate> {
        ["漢字", "感じ", "幹事"].map(Candidate::new).to_vec()
    }

    #[test]
    fn the_most_natural_candidate_comes_first() {
        let mut candidates = kanji();
        ranker(&[-9.0, -8.0, -1.0], 0.0).rank(
            &context("会議の", ""),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(words(&candidates), ["幹事", "感じ", "漢字"]);
    }

    #[test]
    fn the_dictionary_order_holds_unless_the_model_is_sure() {
        // 採点の差が小さければ、辞書の順が勝つ。
        let mut candidates = kanji();
        ranker(&[-5.0, -4.8, -4.9], 1.0).rank(
            &context("会議の", ""),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(words(&candidates), ["漢字", "感じ", "幹事"]);
    }

    #[test]
    fn the_model_sees_the_trimmed_surroundings_and_the_okuri() {
        let r = ranker(&[0.0, 0.0], 1.0);
        let mut candidates = ["送", "贈"].map(Candidate::new).to_vec();
        r.rank(
            &context("昨日は荷物を", "ました。"),
            &Query::okuri_ari("おく", 'r', "り"),
            &mut candidates,
        );
        assert_eq!(
            r.scorer.borrow().seen,
            [(
                "荷物を".to_owned(),
                vec!["送り".to_owned(), "贈り".to_owned()],
                "まし".to_owned()
            )]
        );
    }

    #[test]
    fn nothing_moves_without_surroundings() {
        let r = ranker(&[-9.0, -8.0, -1.0], 0.0);
        let mut candidates = kanji();
        r.rank(
            &context("", ""),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(words(&candidates), ["漢字", "感じ", "幹事"]);
        assert!(r.scorer.borrow().seen.is_empty(), "採点もしない");
    }

    #[test]
    fn a_late_answer_leaves_the_order_alone() {
        let r = ranker(&[-9.0, -8.0, -1.0], 0.0);
        r.scorer.borrow_mut().late = true;
        let mut candidates = kanji();
        r.rank(
            &context("会議の", ""),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(words(&candidates), ["漢字", "感じ", "幹事"]);
    }

    #[test]
    fn the_shared_head_is_split_off() {
        let sequences = vec![vec![1, 5, 6, 7], vec![1, 5, 8], vec![1, 5, 6, 9, 9]];
        let (prefix, suffixes) = split(&sequences).unwrap();
        assert_eq!(prefix, [1, 5]);
        assert_eq!(suffixes, [vec![6, 7], vec![8], vec![6, 9, 9]]);
    }

    #[test]
    fn every_candidate_keeps_at_least_one_token() {
        // 一つの候補がもう一つの頭と同じでも、その候補の残りは空にしない。
        let sequences = vec![vec![1, 5, 6], vec![1, 5, 6, 7]];
        let (prefix, suffixes) = split(&sequences).unwrap();
        assert_eq!(prefix, [1, 5]);
        assert_eq!(suffixes, [vec![6], vec![6, 7]]);
    }
}
