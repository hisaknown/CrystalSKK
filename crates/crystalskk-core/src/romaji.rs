//! ローマ字かな変換。
//!
//! 変換規則は「入力 → 出力かな + 次に持ち越す入力」の三つ組で表す。
//! 持ち越しは `tt` → `っ` + `t` のような促音の表現に使う。
//!
//! **規則表は利用者のファイルから来る** (ADR-0021)。ここは規則表を
//! 持たない。促音や撥音のような「どの配列にもありそうな規則」も含めて、
//! 効いている規則はすべてファイルに書かれている。AZIK などの別配列は、
//! ファイルを差し替えることで対応する。

use std::collections::BTreeMap;

/// ローマ字変換規則の一件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// 打鍵列。
    pub input: String,
    /// 出力するかな。
    pub output: String,
    /// 出力後に入力バッファへ残す打鍵列。
    pub next: String,
}

impl Rule {
    fn new(input: &str, output: &str, next: &str) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            next: next.into(),
        }
    }
}

/// ローマ字変換規則の集合。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomajiTable {
    /// 打鍵列をキーとする。前方一致の判定に順序が必要なため `BTreeMap`。
    rules: BTreeMap<String, Rule>,
}

impl RomajiTable {
    /// 規則列から表を作る。同じ打鍵列が複数あれば後のものが勝つ。
    pub fn from_rules(rules: impl IntoIterator<Item = Rule>) -> Self {
        let rules = rules.into_iter().map(|r| (r.input.clone(), r)).collect();
        Self { rules }
    }

    /// 規則の無い表。何を打ってもかなにならない。
    ///
    /// 設定を受け取る前のエンジンが持つ。**既定の配列として使うものでは
    /// ない。**
    pub fn empty() -> Self {
        Self {
            rules: BTreeMap::new(),
        }
    }

    /// タブ区切りの規則表を読む。
    ///
    /// 一行に `打鍵列 <タブ> 出すかな [<タブ> 続けて打ったことにする打鍵列]`。
    /// Google 日本語入力のローマ字テーブルと同じ形である。`#` で始まる行と
    /// 空行は読み飛ばす。
    ///
    /// 読めない行があれば、**一つ目の誤りの行番号と理由**を返す。一部だけ
    /// 読んで動かすと、どの規則が効いていないのか利用者に分からない。
    pub fn parse(text: &str) -> Result<Self, RomajiError> {
        let mut rules = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line_number = index + 1;
            let error = |message: &str| RomajiError {
                line: line_number,
                message: message.to_owned(),
            };
            // 空白だけの行を読み飛ばすのに trim は使わない。打鍵列には
            // 空白そのものを書ける (`z ` など)。
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            // 字下げ以外の制御文字は、説明の行にも置かせない。見えない文字が
            // 紛れた規則表は、どこが効いていないのか利用者に分からない。
            if line.chars().any(|c| c.is_control() && c != '\t') {
                return Err(error("制御文字は使えません"));
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            let (input, output, next) = match fields.as_slice() {
                [input, output] => (*input, *output, ""),
                [input, output, next] => (*input, *output, *next),
                [_] => return Err(error("タブで区切られていません")),
                _ => return Err(error("欄が多すぎます (三つまで)")),
            };
            if input.is_empty() {
                return Err(error("打鍵列が空です"));
            }
            if output.is_empty() && next.is_empty() {
                return Err(error("出すかなも、続きの打鍵列もありません"));
            }
            rules.push(Rule::new(input, output, next));
        }
        if rules.is_empty() {
            return Err(RomajiError {
                line: 0,
                message: "規則が一つもありません".to_owned(),
            });
        }
        Ok(Self::from_rules(rules))
    }

    fn exact(&self, input: &str) -> Option<&Rule> {
        self.rules.get(input)
    }

    /// `input` を真に延長する規則が存在するか。存在すれば入力の続きを待つ。
    fn has_longer(&self, input: &str) -> bool {
        self.rules
            .range(input.to_owned()..)
            .take_while(|(k, _)| k.starts_with(input))
            .any(|(k, _)| k.len() > input.len())
    }
}

/// 規則表の読めなかったところ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomajiError {
    /// 何行目か。1 から数える。ファイル全体の誤りなら 0。
    pub line: usize,
    /// 理由。利用者に見せる文。
    pub message: String,
}

impl std::fmt::Display for RomajiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 {
            f.write_str(&self.message)
        } else {
            write!(f, "{} 行目: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for RomajiError {}

/// ローマ字入力を受け取り、確定したかなを吐き出す変換器。
///
/// 未確定の打鍵列 ([`RomajiConverter::pending`]) を内部に持つ。これは
/// preedit にそのまま表示される (`k` と打った時点の `k`)。
#[derive(Debug, Clone)]
pub struct RomajiConverter {
    table: RomajiTable,
    pending: String,
}

impl RomajiConverter {
    pub fn new(table: RomajiTable) -> Self {
        Self {
            table,
            pending: String::new(),
        }
    }

    /// 未確定の打鍵列。
    pub fn pending(&self) -> &str {
        &self.pending
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// 未確定の打鍵列から一文字削る。削るものがなければ `false`。
    pub fn backspace(&mut self) -> bool {
        self.pending.pop().is_some()
    }

    /// 一打鍵を与え、確定したかなを返す。確定しなければ空文字列。
    pub fn feed(&mut self, c: char) -> String {
        let mut out = String::new();
        let mut buf = std::mem::take(&mut self.pending);
        buf.push(c);

        loop {
            // 完全一致があり、かつこれ以上延長できないなら確定。
            if let Some(rule) = self.table.exact(&buf) {
                if !self.table.has_longer(&buf) {
                    out.push_str(&rule.output);
                    buf = rule.next.clone();
                    if buf.is_empty() {
                        break;
                    }
                    continue;
                }
                // 延長の余地があるので続きを待つ (`n` に対する `na` など)。
                break;
            }
            if self.table.has_longer(&buf) {
                break;
            }

            // ここから先は行き止まり。打鍵列を切り詰めて解釈し直す。
            match self.resolve_dead_end(&buf) {
                Some((emitted, rest)) => {
                    out.push_str(&emitted);
                    if rest.is_empty() {
                        buf.clear();
                        break;
                    }
                    buf = rest;
                }
                None => break,
            }
        }

        self.pending = buf;
        out
    }

    /// 完全一致も延長もない打鍵列を、出力と残りに分解する。
    fn resolve_dead_end(&self, buf: &str) -> Option<(String, String)> {
        let chars: Vec<char> = buf.chars().collect();

        // 1. 最長の真の接頭辞が確定できるなら、そこで切る。
        for len in (1..chars.len()).rev() {
            let head: String = chars[..len].iter().collect();
            if let Some(rule) = self.table.exact(&head) {
                let rest: String = rule
                    .next
                    .chars()
                    .chain(chars[len..].iter().copied())
                    .collect();
                return Some((rule.output.clone(), rest));
            }
        }

        // 促音 (`kk` → `っ` + `k`) や撥音 (`nk` → `ん` + `k`) は、ここでは
        // 扱わない。**規則表に書いてある** (ADR-0021)。ここで補うと、表に
        // 無い規則が効くことになる。

        // 2. 一文字だけなら、変換できない文字としてそのまま出す。
        if chars.len() == 1 {
            return Some((buf.to_owned(), String::new()));
        }

        // 3. 先頭を捨てて解釈し直す。
        Some((String::new(), chars[1..].iter().collect()))
    }

    /// 変換を打ち切り、未確定の打鍵列から取り出せるかなを返す。
    ///
    /// 確定操作やモード切り替えの直前に呼ぶ。`n` は `ん` になり、
    /// それ以外の未確定打鍵は捨てられる。
    pub fn flush(&mut self) -> String {
        let kana = self.take_pending_kana().unwrap_or_default();
        self.pending.clear();
        kana
    }

    /// 未確定の打鍵列が、それだけでかなになるなら、そのかな。
    ///
    /// 判定は規則表に委ねる。SKK の配列は撥音を明示する `n'` のような
    /// 終端付きの規則を持つので、打鍵列に終端を足した規則があるかどうかで
    /// 「単独で成立するか」が決まる。`n` は `ん` になり、`k` は何にもならない。
    pub fn pending_kana(&self) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        let terminated = format!("{}{TERMINATOR}", self.pending);
        self.table
            .exact(&terminated)
            .map(|rule| rule.output.clone())
    }

    /// 単独でかなになる未確定打鍵を取り出す。ならないときは何も変えない。
    ///
    /// 「シフトを押す直前までの打鍵をどう扱うか」を決めるのに使う。
    /// 取り出せなければ打鍵列は残るので、続く打鍵と組み合わせられる。
    pub fn take_pending_kana(&mut self) -> Option<String> {
        let kana = self.pending_kana()?;
        self.pending.clear();
        Some(kana)
    }
}

/// 打鍵列がそこで終わることを示す文字。規則表の `n'` などに使われている。
const TERMINATOR: char = '\'';

#[cfg(test)]
mod tests {
    use super::*;

    /// 同梱の雛形。**試験はこれを規則表として使う。** 雛形と変換器の
    /// 食い違いも、ここで見つかる。
    const TEMPLATE: &str = include_str!("../../crystalskk-settings/romaji.txt");

    fn converter() -> RomajiConverter {
        RomajiConverter::new(RomajiTable::parse(TEMPLATE).expect("雛形は読める"))
    }

    /// 打鍵列を与え、確定したかなを連結して返す。未確定分は捨てる。
    fn typed(keys: &str) -> String {
        let mut c = converter();
        keys.chars().map(|k| c.feed(k)).collect()
    }

    /// 打鍵列を与え、`(確定したかな, 未確定の打鍵列)` を返す。
    fn typed_with_pending(keys: &str) -> (String, String) {
        let mut c = converter();
        let out: String = keys.chars().map(|k| c.feed(k)).collect();
        (out, c.pending().to_owned())
    }

    #[test]
    fn basic_syllables() {
        assert_eq!(typed("aiueo"), "あいうえお");
        assert_eq!(typed("kakikukeko"), "かきくけこ");
        assert_eq!(typed("kanji"), "かんじ");
    }

    #[test]
    fn youon() {
        assert_eq!(typed("kyakyukyo"), "きゃきゅきょ");
        assert_eq!(typed("shashishusho"), "しゃししゅしょ");
        assert_eq!(typed("jugemu"), "じゅげむ");
    }

    #[test]
    fn sokuon_from_doubled_consonant() {
        assert_eq!(typed("kitte"), "きって");
        assert_eq!(typed("gakkou"), "がっこう");
        assert_eq!(typed("issho"), "いっしょ");
        assert_eq!(typed("motto"), "もっと");
    }

    #[test]
    fn explicit_sokuon() {
        assert_eq!(typed("xtu"), "っ");
        assert_eq!(typed("xtsu"), "っ");
    }

    #[test]
    fn hatsuon() {
        // `nn` と `n'` は明示的な撥音。
        assert_eq!(typed("nn"), "ん");
        assert_eq!(typed("n'"), "ん");
        // 子音が続けば `n` 単独でも撥音になる。
        assert_eq!(typed("nk"), "ん");
        assert_eq!(typed("hon"), "ほ");
        assert_eq!(typed("honda"), "ほんだ");
        assert_eq!(typed("sinbun"), "しんぶ");
        // `ny` は `にゃ行` の途中なので撥音にしない。
        assert_eq!(typed("nyanko"), "にゃんこ");
    }

    #[test]
    fn pending_is_kept_for_incomplete_input() {
        assert_eq!(typed_with_pending("k"), (String::new(), "k".to_owned()));
        assert_eq!(typed_with_pending("ky"), (String::new(), "ky".to_owned()));
        assert_eq!(typed_with_pending("kak"), ("か".to_owned(), "k".to_owned()));
        // 促音は `っ` を出した上で子音を持ち越す。
        assert_eq!(typed_with_pending("kk"), ("っ".to_owned(), "k".to_owned()));
        // `n` 単独は撥音か `な行` か決まらないので保留。
        assert_eq!(typed_with_pending("n"), (String::new(), "n".to_owned()));
    }

    #[test]
    fn pending_kana_tells_what_can_stand_alone() {
        let mut c = converter();
        assert_eq!(c.pending_kana(), None, "未確定がなければ何もない");

        c.feed('n');
        assert_eq!(c.pending_kana().as_deref(), Some("ん"));
        // 覗くだけでは打鍵列は消えない。
        assert_eq!(c.pending(), "n");

        c.clear();
        c.feed('k');
        assert_eq!(c.pending_kana(), None, "`k` は単独では成立しない");

        c.clear();
        c.feed('k');
        c.feed('y');
        assert_eq!(c.pending_kana(), None);
    }

    #[test]
    fn take_pending_kana_leaves_unresolvable_input_alone() {
        let mut c = converter();
        c.feed('k');
        assert_eq!(c.take_pending_kana(), None);
        assert_eq!(c.pending(), "k", "取り出せないなら残す");
        // 残っているので続きと組み合わせられる。
        assert_eq!(c.feed('a'), "か");

        let mut c = converter();
        c.feed('n');
        assert_eq!(c.take_pending_kana().as_deref(), Some("ん"));
        assert!(c.is_empty());
    }

    #[test]
    fn flush_resolves_trailing_n_only() {
        let mut c = converter();
        for k in "hon".chars() {
            c.feed(k);
        }
        assert_eq!(c.flush(), "ん");
        assert!(c.is_empty());

        let mut c = converter();
        c.feed('k');
        assert_eq!(c.flush(), "");
        assert!(c.is_empty());
    }

    #[test]
    fn backspace_removes_one_keystroke() {
        let mut c = converter();
        c.feed('k');
        c.feed('y');
        assert_eq!(c.pending(), "ky");
        assert!(c.backspace());
        assert_eq!(c.pending(), "k");
        assert!(c.backspace());
        assert!(!c.backspace());
    }

    #[test]
    fn punctuation_and_symbols() {
        assert_eq!(typed("a-i,u.e"), "あーい、う。え");
        assert_eq!(typed("z-z.zl"), "〜…→");
        assert_eq!(typed("[a]"), "「あ」");
    }

    #[test]
    fn unknown_characters_pass_through() {
        assert_eq!(typed("123"), "123");
        assert_eq!(typed("a1b"), "あ1");
    }

    #[test]
    fn table_can_be_replaced() {
        let table = RomajiTable::from_rules([
            Rule::new("a", "ア", ""),
            Rule::new("kk", "ッ", "k"),
            Rule::new("ka", "カ", ""),
        ]);
        let mut c = RomajiConverter::new(table);
        let out: String = "kkaa".chars().map(|k| c.feed(k)).collect();
        assert_eq!(out, "ッカア");
    }

    #[test]
    fn a_table_reads_like_google_japanese_input() {
        let table = RomajiTable::parse("ka\tか\nkk\tっ\tk\n# 説明\n\nz \t　\n").unwrap();
        let mut c = RomajiConverter::new(table);
        let out: String = "kka".chars().map(|k| c.feed(k)).collect();
        assert_eq!(out, "っか");
        assert_eq!(c.feed('z'), "");
        assert_eq!(c.feed(' '), "　", "空白の打鍵列を切り詰めない");
    }

    #[test]
    fn windows_line_endings_are_accepted() {
        assert!(RomajiTable::parse("ka\tか\r\nki\tき\r\n").is_ok());
    }

    #[test]
    fn a_broken_line_is_named() {
        let error = RomajiTable::parse("ka\tか\nki き\n").unwrap_err();
        assert_eq!(error.line, 2);
        assert!(error.to_string().contains("2 行目"));

        let error = RomajiTable::parse("ka\tか\tk\tx\n").unwrap_err();
        assert!(error.message.contains("多すぎ"));
        assert!(RomajiTable::parse("\tか\n").is_err(), "打鍵列が空");
        assert!(RomajiTable::parse("# 空っぽ\n").is_err(), "規則が無い");
    }

    #[test]
    fn nothing_is_filled_in_behind_the_table() {
        // **表に無い規則は効かない。** 促音も撥音も、表に書かれていなければ
        // 起きない。
        let table = RomajiTable::parse("ka\tか\nta\tた\n").unwrap();
        let mut c = RomajiConverter::new(table);
        let out: String = "kka".chars().map(|k| c.feed(k)).collect();
        assert_eq!(out, "か", "っ は出ない");
    }

    #[test]
    fn an_empty_table_makes_no_kana() {
        let mut c = RomajiConverter::new(RomajiTable::empty());
        assert_eq!(c.feed('a'), "a");
    }
}
