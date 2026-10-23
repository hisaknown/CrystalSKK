//! ネットの候補 (ADR-0042)。
//!
//! 辞書の候補が尽きたときに、TIP から頼まれて引く。**頼みには待たずに
//! 答える。** 外の API を待てばパイプの列が塞がり、ほかのアプリの変換まで
//! 止まる (ADR-0032)。取りに行くのは裏のスレッドで、取れたものは読みごとに
//! 覚えておく。無かったことも覚え、二度目からは外へ出ない。
//!
//! 覚えるのはサーバが動いているあいだだけで、ファイルには書かない。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;

use crystalskk_core::dict::{Candidate, Query};
use crystalskk_core::kana;

/// 読みを渡し、候補を取ってくるもの。裏のスレッドで呼ぶ。
pub type Fetch = Arc<dyn Fn(&Query) -> Result<Vec<Candidate>, String> + Send + Sync>;

/// ネットの候補を取りに行き、覚えておく係。
pub struct WebSource {
    fetch: Fetch,
    /// 尋ねた読みと、その様子。`None` は取りに行っている最中。
    known: HashMap<Query, Option<Vec<Candidate>>>,
    sender: mpsc::Sender<(Query, Vec<Candidate>)>,
    receiver: mpsc::Receiver<(Query, Vec<Candidate>)>,
}

impl std::fmt::Debug for WebSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSource")
            .field("known", &self.known.len())
            .finish_non_exhaustive()
    }
}

impl WebSource {
    pub fn new(fetch: Fetch) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            fetch,
            known: HashMap::new(),
            sender,
            receiver,
        }
    }

    /// 候補を引く。まだ取れていなければ `None` を返し、裏で取りに行く。
    pub fn lookup(&mut self, query: &Query) -> Option<Vec<Candidate>> {
        self.poll();
        if let Some(known) = self.known.get(query) {
            return known.clone();
        }
        self.known.insert(query.clone(), None);
        let fetch = Arc::clone(&self.fetch);
        let sender = self.sender.clone();
        let query = query.clone();
        std::thread::spawn(move || {
            // 取れなかったときも「無かった」として覚える。辞書登録へ
            // 進めなくなるより、登録できるほうがよい。
            let found = fetch(&query).unwrap_or_else(|e| {
                eprintln!("crystalskk-server: ネットの候補を取れませんでした: {e}");
                Vec::new()
            });
            let _ = sender.send((query, found));
        });
        None
    }

    /// 裏で取れたものを受け取る。
    fn poll(&mut self) {
        while let Ok((query, found)) = self.receiver.try_recv() {
            self.known.insert(query, Some(found));
        }
    }
}

/// ネットの候補に付ける注釈。
pub const MARK: &str = "🌐";

/// Google CGI API for Japanese Input に尋ねる。
pub fn google(query: &Query) -> Result<Vec<Candidate>, String> {
    let (text, okuri) = reading(query);
    let url = format!(
        "https://www.google.com/transliterate?langpair=ja-Hira%7Cja&text={}",
        percent_encode(&format!("{text},"))
    );
    let body = match crystalskk_fetch::get(&url, None).map_err(|e| e.to_string())? {
        crystalskk_fetch::Fetched::Downloaded(downloaded) => downloaded.body,
        crystalskk_fetch::Fetched::NotModified => return Ok(Vec::new()),
    };
    let body = String::from_utf8(body).map_err(|_| "答えが UTF-8 ではありません".to_owned())?;
    let segments = parse_segments(&body).ok_or("答えを読めません")?;
    Ok(candidates(&segments, &text, okuri.as_deref()))
}

/// 尋ねる読み。送りありなら送り仮名まで含め、送り仮名も返す。
///
/// 辞書キーの末尾の子音 (`おくr`) は向こうには通じないので外す。
fn reading(query: &Query) -> (String, Option<String>) {
    match &query.okuri {
        Some(okuri) => {
            let mut midashi = query.key.clone();
            if midashi.ends_with(|c: char| c.is_ascii_alphabetic()) {
                midashi.pop();
            }
            (format!("{midashi}{okuri}"), Some(okuri.clone()))
        }
        None => (query.key.clone(), None),
    }
}

/// 答えの文節から候補を作る。
///
/// 一つの文節なら、その候補をそのまま。分けられたなら、各文節の一番目を
/// つないだ一つだけにする。末尾に付けた `,` の文節は捨てる (読みを一つの
/// 文節に留めるために付けたもの)。送りありなら、送り仮名で終わるものだけ
/// を残し、送り仮名を外す。
///
/// **読みをそのまま、またはカタカナにしただけのものは除く。** ひらがなは
/// 確定すれば、カタカナは `q` で打てる。候補に並べても選ぶ手間が増えるだけ
/// である。
fn candidates(
    segments: &[(String, Vec<String>)],
    reading: &str,
    okuri: Option<&str>,
) -> Vec<Candidate> {
    let obvious = [
        reading.to_owned(),
        kana::to_katakana(reading),
        kana::to_halfwidth_katakana(reading),
    ];
    let segments: Vec<_> = segments
        .iter()
        .filter(|(reading, _)| reading != ",")
        .collect();
    let words: Vec<String> = match segments.as_slice() {
        [] => Vec::new(),
        [(_, words)] => words.clone(),
        many => many
            .iter()
            .map(|(_, words)| words.first().cloned())
            .collect::<Option<String>>()
            .into_iter()
            .collect(),
    };
    let mut out: Vec<Candidate> = Vec::new();
    for word in words {
        if obvious.contains(&word) {
            continue;
        }
        let word = match okuri {
            Some(okuri) => match word.strip_suffix(okuri) {
                Some(stem) if !stem.is_empty() => stem.to_owned(),
                _ => continue,
            },
            None => word,
        };
        if !out.iter().any(|c| c.word == word) {
            out.push(Candidate::with_annotation(word, MARK));
        }
    }
    out
}

/// URL に入れるため、ASCII の英数字と一部の記号のほかを `%XX` にする。
fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `[["読み",["候補",...]],...]` を読む。
///
/// 答えの形はこれだけなので、JSON の読み手を足さずに済ませる。読めない
/// 形なら `None`。
fn parse_segments(text: &str) -> Option<Vec<(String, Vec<String>)>> {
    let mut p = Parser {
        chars: text.chars().peekable(),
    };
    let mut segments = Vec::new();
    p.expect('[')?;
    if p.eat(']') {
        return Some(segments);
    }
    loop {
        p.expect('[')?;
        let reading = p.string()?;
        p.expect(',')?;
        p.expect('[')?;
        let mut words = Vec::new();
        if !p.eat(']') {
            loop {
                words.push(p.string()?);
                if p.eat(']') {
                    break;
                }
                p.expect(',')?;
            }
        }
        p.expect(']')?;
        segments.push((reading, words));
        if p.eat(']') {
            return Some(segments);
        }
        p.expect(',')?;
    }
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.chars.peek().is_some_and(|c| c.is_whitespace()) {
            self.chars.next();
        }
    }

    fn eat(&mut self, wanted: char) -> bool {
        self.skip_space();
        if self.chars.peek() == Some(&wanted) {
            self.chars.next();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, wanted: char) -> Option<()> {
        self.eat(wanted).then_some(())
    }

    fn string(&mut self) -> Option<String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            match self.chars.next()? {
                '"' => return Some(out),
                '\\' => match self.chars.next()? {
                    'u' => {
                        let unit = self.hex4()?;
                        let c = if (0xD800..0xDC00).contains(&unit) {
                            // サロゲートの対。
                            if self.chars.next()? != '\\' || self.chars.next()? != 'u' {
                                return None;
                            }
                            let low = self.hex4()?;
                            char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))?
                        } else {
                            char::from_u32(unit)?
                        };
                        out.push(c);
                    }
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    c => out.push(c),
                },
                c => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let mut value = 0;
        for _ in 0..4 {
            value = value * 16 + self.chars.next()?.to_digit(16)?;
        }
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn words(candidates: &[Candidate]) -> Vec<&str> {
        candidates.iter().map(|c| c.word.as_str()).collect()
    }

    #[test]
    fn an_answer_is_read() {
        let segments =
            parse_segments(r#"[["かんじ",["漢字","感じ","幹事"]],[",",[","]]]"#).unwrap();
        assert_eq!(
            words(&candidates(&segments, "かんじ", None)),
            ["漢字", "感じ", "幹事"]
        );
    }

    #[test]
    fn every_candidate_is_marked() {
        let segments = parse_segments(r#"[["かんじ",["漢字"]]]"#).unwrap();
        assert_eq!(
            candidates(&segments, "かんじ", None),
            [Candidate::with_annotation("漢字", MARK)]
        );
    }

    #[test]
    fn okuri_is_cut_off_and_must_match() {
        let segments = parse_segments(r#"[["おくり",["送り","贈り","送"]]]"#).unwrap();
        assert_eq!(
            words(&candidates(&segments, "おくり", Some("り"))),
            ["送", "贈"]
        );
    }

    #[test]
    fn split_segments_become_one_word() {
        let segments =
            parse_segments(r#"[["くりすたる",["クリスタル"]],["すけ",["助","すけ"]]]"#).unwrap();
        assert_eq!(
            words(&candidates(&segments, "くりすたるすけ", None)),
            ["クリスタル助"]
        );
    }

    #[test]
    fn the_reading_in_kana_is_left_out() {
        let segments =
            parse_segments(r#"[["くりすたる",["クリスタル","Crystal","くりすたる","ｸﾘｽﾀﾙ"]]]"#)
                .unwrap();
        assert_eq!(
            words(&candidates(&segments, "くりすたる", None)),
            ["Crystal"],
            "ひらがな・カタカナ・半角カタカナは打てば出る"
        );
    }

    #[test]
    fn a_broken_answer_is_refused() {
        assert_eq!(parse_segments("<html>"), None);
        assert_eq!(parse_segments(r#"[["かんじ""#), None);
    }

    #[test]
    fn the_reading_drops_the_okuri_head() {
        let query = Query::okuri_ari("おく", 'r', "り");
        assert_eq!(
            reading(&query),
            ("おくり".to_owned(), Some("り".to_owned()))
        );
    }

    #[test]
    fn the_reading_is_percent_encoded() {
        assert_eq!(percent_encode("か,"), "%E3%81%8B%2C");
    }

    fn wait_for(web: &mut WebSource, query: &Query) -> Vec<Candidate> {
        let start = Instant::now();
        loop {
            if let Some(found) = web.lookup(query) {
                return found;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "取れるはず");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_first_answer_is_pending_and_later_ones_are_remembered() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let mut web = WebSource::new(Arc::new(move |_query: &Query| {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(vec![Candidate::new("漢字")])
        }));
        let query = Query::okuri_nashi("かんじ");

        assert_eq!(web.lookup(&query), None, "待たずに答える");
        assert_eq!(words(&wait_for(&mut web, &query)), ["漢字"]);
        assert_eq!(words(&web.lookup(&query).unwrap()), ["漢字"]);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "二度目からは外へ出ない");
    }

    #[test]
    fn a_failure_is_remembered_as_nothing() {
        let mut web = WebSource::new(Arc::new(|_query: &Query| Err("繋がらない".to_owned())));
        let query = Query::okuri_nashi("かんじ");
        assert_eq!(web.lookup(&query), None);
        assert!(wait_for(&mut web, &query).is_empty());
    }
}
