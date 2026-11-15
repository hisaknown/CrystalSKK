//! 変換の候補を、かな漢字変換の言語モデルで前の文章から並べる (ADR-0030, ADR-0043)。
//!
//! # 採点は生成ではない
//!
//! モデルは jinen (かな漢字変換のために学習した小さなモデル) で、次の形の
//! 列を学んでいる。
//!
//! ```text
//! U+EE02 前の文章 U+EE00 読み (カタカナ) U+EE01 変換結果 </s>
//! ```
//!
//! `U+EE01` までを全候補で共有して一度だけ計算し、各候補の「変換結果 `</s>`」
//! の確率を一つのバッチで求めて比べる。`</s>` まで含めるので、候補が
//! 「そこで終わる」ことも確率に入る。後ろの文章はモデルが受け取らない。
//!
//! # 自信があるときだけ、一つだけ持ち上げる
//!
//! 採点を候補のあいだで確率に直し (softmax)、一番の候補の確率が
//! `confidence` 以上なら、それだけを先頭に出す。残りは辞書の順のままに
//! する。2 位以下の採点はモデルにとって手がかりが無く、ほぼ雑音である。
//! 採点と辞書の順を足し合わせると、その雑音が 2 位以下の並びに乗る
//! (ADR-0044)。持ち上げた候補には印
//! ([`crystalskk_core::dict::RANKED_MARK`]) を付ける。
//!
//! # 当てにならないときは並べない
//!
//! 並べ替えるのは辞書の上位だけである。`ln(順位)` は後ろのほうでほとんど
//! 差がつかず、候補が多いほど、たまたま良い点を取る外れも出やすい。
//! 前の文章がほとんど無いときも並べない。採点が「文脈抜きで、ありふれた
//! 変換か」になり、辞書の順より当たらない。
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
const MAX_CONTEXT_TOKENS: usize = 256;

/// 候補を採点するもの。
pub trait Scorer {
    /// 前の文章 `before` に続けて読み `reading` を打ったとき、それが各
    /// `texts` に変換される確率の対数。大きいほど自然。締め切りに間に合わ
    /// なければ `None`。
    fn score(
        &mut self,
        before: &str,
        reading: &str,
        texts: &[String],
        deadline: Instant,
    ) -> Option<Vec<f32>>;
}

/// llama.cpp で動く言語モデル。
#[derive(Debug)]
pub struct LlamaScorer {
    model: llama::Model,
    tokenizer: Tokenizer,
    marks: Marks,
}

/// プロンプトを組む特殊トークン。
#[derive(Debug, Clone, Copy)]
struct Marks {
    context: i32,
    input: i32,
    output: i32,
    end: i32,
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
        let path = tokenizer;
        let tokenizer = Tokenizer::from_file(path)
            .map_err(|e| format!("{} を語彙として読めません: {e}", path.display()))?;
        let id = |token: &str| {
            tokenizer
                .token_to_id(token)
                .map(|id| id as i32)
                .ok_or_else(|| format!("{} に {token:?} がありません", path.display()))
        };
        let marks = Marks {
            context: id("\u{ee02}")?,
            input: id("\u{ee00}")?,
            output: id("\u{ee01}")?,
            end: id("</s>")?,
        };
        let model = llama::Model::load(runtime, model, threads)?;
        Ok(Self {
            model,
            tokenizer,
            marks,
        })
    }

    /// 特殊トークンを付けずにトークンに分ける。NFKC への正規化は語彙が持って
    /// いる。
    fn encode(&self, text: &str) -> Option<Vec<i32>> {
        if text.is_empty() {
            return Some(Vec::new());
        }
        let encoding = self.tokenizer.encode(text, false).ok()?;
        Some(encoding.get_ids().iter().map(|&id| id as i32).collect())
    }
}

impl Scorer for LlamaScorer {
    fn score(
        &mut self,
        before: &str,
        reading: &str,
        texts: &[String],
        deadline: Instant,
    ) -> Option<Vec<f32>> {
        let Marks {
            context,
            input,
            output,
            end,
        } = self.marks;
        let mut prompt = Vec::new();
        let before = self.encode(before)?;
        if !before.is_empty() {
            // 前の文章が長すぎれば、古いほうを落とす。
            prompt.push(context);
            prompt.extend_from_slice(&before[before.len().saturating_sub(MAX_CONTEXT_TOKENS)..]);
        }
        prompt.push(input);
        prompt.extend(self.encode(&katakana(reading))?);
        prompt.push(output);
        let suffixes = texts
            .iter()
            .map(|text| {
                let mut ids = self.encode(text)?;
                ids.push(end);
                Some(ids)
            })
            .collect::<Option<Vec<_>>>()?;
        self.model.score(&prompt, &suffixes, deadline)
    }
}

/// ひらがなをカタカナにする。モデルは読みをカタカナで学んでいる。
fn katakana(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'ぁ'..='ゖ' | 'ゝ' | 'ゞ' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// 問い合わせの読み。送りありなら、見出しの末尾の子音を送り仮名に替える
/// (`おくr` と `り` から `おくり`)。候補も送り仮名を付けて採点する。
fn reading(query: &Query) -> String {
    match &query.okuri {
        Some(okuri) => {
            let mut stem = query.key.clone();
            stem.pop();
            stem + okuri
        }
        None => query.key.clone(),
    }
}

/// 並べ方の決まり。
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    /// 一番の候補を持ち上げるのに要る、候補のあいだでの確率。
    pub confidence: f32,
    /// 並べ替えを待つ時間。
    pub deadline: Duration,
    /// 前の文章を何文字見せるか。
    pub before: usize,
    /// 辞書の順で上から何個までを並べ替えるか。
    pub top: usize,
    /// 前の文章に、句読点や空白を除いて何文字あれば並べ替えるか。
    pub min_context: usize,
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
        let top = self.policy.top.min(candidates.len());
        if top < 2 {
            return;
        }
        let before = context.text_before(self.policy.before);
        // 手がかりがほとんど無ければ、辞書の順より良くなる見込みは薄い。
        if before.is_empty() || clues(&before) < self.policy.min_context {
            return;
        }
        let mut rest = candidates.split_off(top);
        self.rank_head(&before, query, candidates);
        candidates.append(&mut rest);
    }
}

/// 手がかりになる文字の数。句読点・記号・空白は数えない。
fn clues(before: &str) -> usize {
    before.chars().filter(|c| c.is_alphanumeric()).count()
}

impl<S: Scorer> LmRanker<S> {
    fn rank_head(&self, before: &str, query: &Query, candidates: &mut Vec<Candidate>) {
        let texts: Vec<String> = candidates
            .iter()
            .map(|c| c.to_text(query.okuri.as_deref()))
            .collect();
        let deadline = Instant::now() + self.policy.deadline;
        let Some(scores) =
            self.scorer
                .borrow_mut()
                .score(before, &reading(query), &texts, deadline)
        else {
            return;
        };
        if scores.len() != candidates.len() || Instant::now() > deadline {
            return;
        }
        promote(candidates, &scores, self.policy.confidence);
    }
}

/// 一番の候補の、候補のあいだでの確率が `confidence` 以上なら、それを
/// 先頭に出して印を付ける。残りは辞書の順のまま。
fn promote(candidates: &mut Vec<Candidate>, scores: &[f32], confidence: f32) {
    let Some((best, &top)) = scores.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) else {
        return;
    };
    if best == 0 || !top.is_finite() {
        return;
    }
    let total: f32 = scores.iter().map(|s| (s - top).exp()).sum();
    if 1.0 / total < confidence {
        return;
    }
    let mut chosen = candidates.remove(best);
    chosen.mark_ranked();
    candidates.insert(0, chosen);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 決まった点を返す採点係。見せられた文章と読みを覚えておく。
    struct Fixed {
        scores: Vec<f32>,
        seen: Vec<(String, String, Vec<String>)>,
        late: bool,
    }

    impl Scorer for Fixed {
        fn score(
            &mut self,
            before: &str,
            reading: &str,
            texts: &[String],
            _: Instant,
        ) -> Option<Vec<f32>> {
            self.seen
                .push((before.to_owned(), reading.to_owned(), texts.to_vec()));
            (!self.late).then(|| self.scores.clone())
        }
    }

    fn ranker(scores: &[f32], confidence: f32) -> LmRanker<Fixed> {
        LmRanker::new(
            Fixed {
                scores: scores.to_vec(),
                seen: Vec::new(),
                late: false,
            },
            Policy {
                confidence,
                deadline: Duration::from_secs(1),
                before: 3,
                top: 3,
                min_context: 1,
            },
        )
    }

    fn words(candidates: &[Candidate]) -> Vec<&str> {
        candidates.iter().map(|c| c.word.as_str()).collect()
    }

    fn context(before: &str) -> Context {
        Context {
            preceding_text: Some(before.to_owned()),
            following_text: Some("後ろの文章".to_owned()),
            ..Context::default()
        }
    }

    fn kanji() -> Vec<Candidate> {
        ["漢字", "感じ", "幹事"].map(Candidate::new).to_vec()
    }

    #[test]
    fn the_most_natural_candidate_comes_first() {
        let mut candidates = kanji();
        ranker(&[-9.0, -8.0, -1.0], 0.7).rank(
            &context("会議の"),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(
            words(&candidates),
            ["幹事", "漢字", "感じ"],
            "残りは辞書の順"
        );
        assert_eq!(candidates[0].annotation.as_deref(), Some("✦"), "印が付く");
        assert_eq!(candidates[1].annotation, None);
    }

    #[test]
    fn the_dictionary_order_holds_unless_the_model_is_sure() {
        // 一番でも、確率が足りなければ動かさない。
        let mut candidates = kanji();
        ranker(&[-5.0, -4.8, -4.9], 0.7).rank(
            &context("会議の"),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(words(&candidates), ["漢字", "感じ", "幹事"]);
        assert!(
            candidates.iter().all(|c| c.annotation.is_none()),
            "印も付かない"
        );
    }

    #[test]
    fn the_first_stays_unmarked_when_the_model_agrees() {
        let mut candidates = kanji();
        ranker(&[-0.1, -8.0, -9.0], 0.7).rank(
            &context("会議の"),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(words(&candidates), ["漢字", "感じ", "幹事"]);
        assert_eq!(candidates[0].annotation, None);
    }

    #[test]
    fn the_model_sees_the_trimmed_text_before_and_the_whole_reading() {
        let r = ranker(&[0.0, 0.0], 1.0);
        let mut candidates = ["送", "贈"].map(Candidate::new).to_vec();
        r.rank(
            &context("昨日は荷物を"),
            &Query::okuri_ari("おく", 'r', "り"),
            &mut candidates,
        );
        assert_eq!(
            r.scorer.borrow().seen,
            [(
                "荷物を".to_owned(),
                "おくり".to_owned(),
                vec!["送り".to_owned(), "贈り".to_owned()],
            )]
        );
    }

    #[test]
    fn nothing_moves_without_text_before() {
        let r = ranker(&[-9.0, -8.0, -1.0], 0.0);
        let mut candidates = kanji();
        r.rank(&context(""), &Query::okuri_nashi("かんじ"), &mut candidates);
        assert_eq!(words(&candidates), ["漢字", "感じ", "幹事"]);
        assert!(r.scorer.borrow().seen.is_empty(), "採点もしない");
    }

    #[test]
    fn nothing_moves_with_only_punctuation_before() {
        let r = ranker(&[-9.0, -8.0, -1.0], 0.0);
        let mut candidates = kanji();
        r.rank(
            &context("。\n「"),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(words(&candidates), ["漢字", "感じ", "幹事"]);
        assert!(r.scorer.borrow().seen.is_empty(), "採点もしない");
    }

    #[test]
    fn only_the_top_of_the_dictionary_is_reordered() {
        let r = ranker(&[-9.0, -8.0, -1.0], 0.0);
        let mut candidates = ["漢字", "感じ", "幹事", "監事", "寛治"]
            .map(Candidate::new)
            .to_vec();
        r.rank(
            &context("会議の"),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(words(&candidates), ["幹事", "漢字", "感じ", "監事", "寛治"]);
        assert_eq!(
            r.scorer.borrow().seen[0].2,
            ["漢字", "感じ", "幹事"],
            "上位だけ採点する"
        );
    }

    #[test]
    fn a_late_answer_leaves_the_order_alone() {
        let r = ranker(&[-9.0, -8.0, -1.0], 0.0);
        r.scorer.borrow_mut().late = true;
        let mut candidates = kanji();
        r.rank(
            &context("会議の"),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(words(&candidates), ["漢字", "感じ", "幹事"]);
    }

    #[test]
    fn the_reading_is_given_in_katakana() {
        assert_eq!(katakana("おくり"), "オクリ");
        assert_eq!(katakana("ゔぁいおりん"), "ヴァイオリン");
        assert_eq!(katakana("すゝむ"), "スヽム");
        assert_eq!(katakana("CPUー"), "CPUー");
    }
}
