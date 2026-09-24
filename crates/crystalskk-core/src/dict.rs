//! 候補の供給と並び替え。
//!
//! 辞書の実体 (ファイル、ネットワーク、ユーザー辞書) はこのクレートには
//! 存在しない。エンジンは [`CandidateSource`] と [`Ranker`] という二つの
//! 差し込み口だけを知っている。
//!
//! この二つを分けてあるのは、「候補を増やす」拡張と「候補の順を変える」
//! 拡張が別物だからである。補完や予測変換は前者、学習や文脈スコアリングは
//! 後者として、互いに影響せず足していける。

use crate::InputMode;

/// 辞書を引くための問い合わせ。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Query {
    /// 辞書キー。送りありなら末尾に送り仮名の子音を含む (`おくr`)。
    pub key: String,
    /// 送り仮名。送りなしなら `None`。
    pub okuri: Option<String>,
}

impl Query {
    /// 送りなしの問い合わせ。
    pub fn okuri_nashi(midashi: impl Into<String>) -> Self {
        Self {
            key: midashi.into(),
            okuri: None,
        }
    }

    /// 送りありの問い合わせ。`okuri_head` は送り仮名の最初のローマ字。
    pub fn okuri_ari(midashi: &str, okuri_head: char, okuri: impl Into<String>) -> Self {
        Self {
            key: format!("{midashi}{okuri_head}"),
            okuri: Some(okuri.into()),
        }
    }

    pub fn is_okuri_ari(&self) -> bool {
        self.okuri.is_some()
    }
}

/// 辞書が返す候補一件。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Candidate {
    /// 変換結果。送り仮名は含まない。
    pub word: String,
    /// 注釈。辞書上で `;` に続く部分。
    pub annotation: Option<String>,
}

impl Candidate {
    pub fn new(word: impl Into<String>) -> Self {
        Self {
            word: word.into(),
            annotation: None,
        }
    }

    pub fn with_annotation(word: impl Into<String>, annotation: impl Into<String>) -> Self {
        Self {
            word: word.into(),
            annotation: Some(annotation.into()),
        }
    }

    /// 送り仮名を付けた、実際に確定される文字列。
    pub fn to_text(&self, okuri: Option<&str>) -> String {
        match okuri {
            Some(o) => format!("{}{o}", self.word),
            None => self.word.clone(),
        }
    }
}

/// 候補を供給するもの。静的辞書、ユーザー辞書、補完、予測などが実装する。
pub trait CandidateSource {
    /// 問い合わせに対する候補を、そのソースにとって自然な順で返す。
    ///
    /// 並び順の最終決定は [`Ranker`] が行うので、ここでは順位付けに悩まなくてよい。
    fn lookup(&self, query: &Query) -> Vec<Candidate>;

    /// 変換のために引く。周辺情報を添える。
    ///
    /// 周辺情報を並びに生かせるソース (候補をプロセスの外で引き、そこで
    /// 並べるもの) だけが上書きする。既定は [`Self::lookup`] と同じ。
    ///
    /// **エンジンが使うのは変換のときだけである。** 補完の見せ方や語の
    /// 有無を確かめるときは [`Self::lookup`] を使い、並べる手間をかけない。
    fn lookup_for_conversion(&self, query: &Query, context: &Context) -> Vec<Candidate> {
        let _ = context;
        self.lookup(query)
    }

    /// 前方一致する見出しを返す。補完に使う。
    ///
    /// 並びはソースに任せる。**使った語を先に出すのは、それを知っている
    /// ソースの仕事**であって、ここで決めることではない。
    ///
    /// 補完を持たないソースは何も返さない。既定がそれである。
    fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        let _ = (prefix, limit);
        Vec::new()
    }

    /// いま引ける状態か。
    ///
    /// **「候補が無い」と「引けなかった」は別である。** 前者なら辞書登録へ
    /// 進むのが正しいが、後者で登録を始めるのは嘘になる。利用者から見れば
    /// 「知っているはずの語が辞書に無いと言われた」ことになり、そのまま
    /// 登録すれば辞書が汚れる。
    ///
    /// 手元の辞書を引くだけのソースは、常に引ける。プロセスの外へ尋ねる
    /// ソースだけがこれを偽にする。
    fn available(&self) -> bool {
        true
    }
}

/// 変換時に参照できる周辺情報。
///
/// 取得できない項目があることを前提とする。TSF はアプリケーションによっては
/// 周辺テキストを返さないため、[`Self::preceding_text`] は常に `None`
/// でありうる。ランカーは欠損しても機能を落とすだけで済むように書くこと。
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// 現在の入力モード。
    pub mode: InputMode,
    /// 直近に確定した文字列。新しいものが先頭。
    pub recent_commits: Vec<String>,
    /// カーソル前のテキスト。取得できなければ `None`。
    pub preceding_text: Option<String>,
    /// カーソル後のテキスト。取得できなければ `None`。
    ///
    /// 文章の末尾に書き足していくのがふつうなので、取れても空のことが多い。
    pub following_text: Option<String>,
    /// 入力先アプリケーションの識別子。取得できなければ `None`。
    pub application: Option<String>,
}

impl Context {
    /// カーソル前の文章を、末尾から `max_chars` 文字まで。
    ///
    /// **取れなければ直近の確定文字列で代える。** 周辺テキストを返さない
    /// アプリでも、この入力で確定したものは分かっている。
    pub fn text_before(&self, max_chars: usize) -> String {
        let text = match &self.preceding_text {
            Some(text) => text.clone(),
            None => self
                .recent_commits
                .iter()
                .rev()
                .map(String::as_str)
                .collect(),
        };
        last_chars(&text, max_chars).to_owned()
    }

    /// カーソル後の文章を、先頭から `max_chars` 文字まで。取れなければ空。
    pub fn text_after(&self, max_chars: usize) -> String {
        let text = self.following_text.as_deref().unwrap_or_default();
        match text.char_indices().nth(max_chars) {
            Some((end, _)) => text[..end].to_owned(),
            None => text.to_owned(),
        }
    }
}

/// 末尾から `max_chars` 文字。
fn last_chars(text: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }
    match text.char_indices().rev().nth(max_chars - 1) {
        Some((start, _)) => &text[start..],
        None => text,
    }
}

/// 候補の提示順を決めるもの。
pub trait Ranker {
    /// 候補列をその場で並び替える。候補の増減は行わない。
    fn rank(&self, context: &Context, query: &Query, candidates: &mut Vec<Candidate>);
}

/// 何もしないランカー。辞書が返した順をそのまま使う。
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopRanker;

impl Ranker for NoopRanker {
    fn rank(&self, _context: &Context, _query: &Query, _candidates: &mut Vec<Candidate>) {}
}

/// 候補を持たない辞書。テストや、辞書の準備ができていない状態で使う。
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyDict;

impl CandidateSource for EmptyDict {
    fn lookup(&self, _query: &Query) -> Vec<Candidate> {
        Vec::new()
    }
}

/// 複数の候補ソースを順に引き、重複を除いて連結するソース。
///
/// 先に登録したソースの候補が先に並ぶ。ユーザー辞書を静的辞書より前に
/// 置く、といった使い方をする。
pub struct ChainedSource {
    sources: Vec<Box<dyn CandidateSource>>,
}

impl ChainedSource {
    pub fn new(sources: Vec<Box<dyn CandidateSource>>) -> Self {
        Self { sources }
    }
}

impl std::fmt::Debug for ChainedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainedSource")
            .field("sources", &self.sources.len())
            .finish()
    }
}

impl CandidateSource for ChainedSource {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = Vec::new();
        for source in &self.sources {
            for candidate in source.lookup(query) {
                if !out.iter().any(|c| c.word == candidate.word) {
                    out.push(candidate);
                }
            }
        }
        out
    }

    /// 前の辞書から順に集める。**並べた順がそのまま出る順になる。**
    fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for source in &self.sources {
            for key in source.complete(prefix, limit) {
                if out.len() >= limit {
                    return out;
                }
                if !out.contains(&key) {
                    out.push(key);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn the_text_before_is_cut_from_its_end() {
        let context = Context {
            preceding_text: Some("今日は会議の".to_owned()),
            ..Context::default()
        };
        assert_eq!(context.text_before(3), "会議の");
        assert_eq!(context.text_before(100), "今日は会議の");
        assert_eq!(context.text_before(0), "");
    }

    #[test]
    fn the_text_after_is_cut_from_its_start() {
        let context = Context {
            following_text: Some("を務めた。".to_owned()),
            ..Context::default()
        };
        assert_eq!(context.text_after(3), "を務め");
        assert_eq!(context.text_after(100), "を務めた。");
        assert_eq!(Context::default().text_after(5), "");
    }

    #[test]
    fn recent_commits_stand_in_oldest_first() {
        let context = Context {
            recent_commits: vec!["会議の".to_owned(), "今日は".to_owned()],
            ..Context::default()
        };
        assert_eq!(context.text_before(100), "今日は会議の");
    }

    struct Fixed(HashMap<String, Vec<Candidate>>);

    impl CandidateSource for Fixed {
        fn lookup(&self, query: &Query) -> Vec<Candidate> {
            self.0.get(&query.key).cloned().unwrap_or_default()
        }
    }

    fn fixed(entries: &[(&str, &[&str])]) -> Fixed {
        Fixed(
            entries
                .iter()
                .map(|(k, words)| {
                    (
                        (*k).to_owned(),
                        words.iter().map(|w| Candidate::new(*w)).collect(),
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn okuri_ari_key_includes_the_okuri_head() {
        let q = Query::okuri_ari("おく", 'r', "り");
        assert_eq!(q.key, "おくr");
        assert_eq!(q.okuri.as_deref(), Some("り"));
        assert!(q.is_okuri_ari());
    }

    #[test]
    fn candidate_text_appends_okuri() {
        let c = Candidate::new("送");
        assert_eq!(c.to_text(Some("り")), "送り");
        assert_eq!(c.to_text(None), "送");
    }

    #[test]
    fn chained_source_keeps_first_occurrence() {
        let user = fixed(&[("かんじ", &["感じ"])]);
        let system = fixed(&[("かんじ", &["漢字", "感じ", "幹事"])]);
        let chained = ChainedSource::new(vec![Box::new(user), Box::new(system)]);

        let got = chained.lookup(&Query::okuri_nashi("かんじ"));
        let words: Vec<&str> = got.iter().map(|c| c.word.as_str()).collect();
        assert_eq!(words, ["感じ", "漢字", "幹事"]);
    }
}
