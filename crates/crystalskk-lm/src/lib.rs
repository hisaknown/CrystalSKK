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
//! # モデルが推す候補だけを、前に差し込む
//!
//! 採点はトークンごとに語彙全体で正規化した対数確率の和なので、`exp` を
//! 取れば「モデルがこの読みからこの候補をそのまま出す確率」になる。その
//! 確率が `confidence` 以上の候補を「モデルが推す候補」とし、モデルの順で
//! 前に並べる。推さない候補は辞書の順のまま後ろに続ける。確率は全候補で
//! 足しても 1 以下なので、推す候補は多くても `1 / confidence` 個である。
//!
//! 候補のあいだで正規化し直すことはしない。どの候補にもモデルが自信を
//! 持っていないときでも、確率が大きく見えるからである。推さない候補の
//! 採点はモデルにとって手がかりが無く、ほぼ雑音なので、並びには使わない
//! (ADR-0044)。
//!
//! 推した候補には、辞書の順のままの位置にあっても印
//! ([`crystalskk_core::dict::RANKED_MARK`]) を付ける。印は「動いた」では
//! なく「モデルが文脈から推している」ことを示す。
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
    /// 候補を推すのに要る、モデルがその候補を出す確率。
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
        endorse(candidates, &scores, self.policy.confidence);
    }
}

/// モデルがそれを出す確率が `confidence` 以上の候補を、モデルの順で前に
/// 並べて印を付ける。残りは辞書の順のまま。
fn endorse(candidates: &mut Vec<Candidate>, scores: &[f32], confidence: f32) {
    let mut endorsed: Vec<usize> = (0..candidates.len())
        .filter(|&i| scores[i].exp() >= confidence)
        .collect();
    if endorsed.is_empty() {
        return;
    }
    // 同点なら辞書の順。
    endorsed.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(a.cmp(&b)));
    let mut slots: Vec<Option<Candidate>> = candidates.drain(..).map(Some).collect();
    for &i in &endorsed {
        if let Some(mut chosen) = slots[i].take() {
            chosen.mark_ranked();
            candidates.push(chosen);
        }
    }
    candidates.extend(slots.into_iter().flatten());
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
        ranker(&[-9.0, -8.0, -0.1], 0.2).rank(
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
        // 一番でも、モデルがそれを出す確率が足りなければ動かさない。
        // 候補のあいだでは一番でも、どれにも自信が無いことがある。
        let mut candidates = kanji();
        ranker(&[-3.0, -9.0, -2.0], 0.2).rank(
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
    fn the_first_is_marked_when_the_model_agrees() {
        // 印は「動いた」ではなく「モデルが推している」こと。
        let mut candidates = kanji();
        ranker(&[-0.1, -8.0, -9.0], 0.2).rank(
            &context("会議の"),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(words(&candidates), ["漢字", "感じ", "幹事"]);
        assert_eq!(candidates[0].annotation.as_deref(), Some("✦"));
        assert_eq!(candidates[1].annotation, None);
    }

    #[test]
    fn every_endorsed_candidate_comes_forward_in_the_models_order() {
        // 0.37 と 0.45 の二つを推し、確率の高い順に前へ。残りは辞書の順。
        let mut candidates = ["漢字", "感じ", "幹事", "監事"]
            .map(Candidate::new)
            .to_vec();
        let r = LmRanker::new(
            Fixed {
                scores: vec![-5.0, -6.0, -1.0, -0.8],
                seen: Vec::new(),
                late: false,
            },
            Policy {
                confidence: 0.2,
                deadline: Duration::from_secs(1),
                before: 3,
                top: 4,
                min_context: 1,
            },
        );
        r.rank(
            &context("会議の"),
            &Query::okuri_nashi("かんじ"),
            &mut candidates,
        );
        assert_eq!(words(&candidates), ["監事", "幹事", "漢字", "感じ"]);
        let marked: Vec<bool> = candidates.iter().map(|c| c.annotation.is_some()).collect();
        assert_eq!(marked, [true, true, false, false]);
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
        let r = ranker(&[-9.0, -8.0, -1.0], 0.3);
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
