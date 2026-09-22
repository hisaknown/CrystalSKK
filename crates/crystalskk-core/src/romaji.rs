//! ローマ字かな変換。
//!
//! 変換規則は「入力 → 出力かな + 次に持ち越す入力」の三つ組で表す。
//! 持ち越しは `tt` → `っ` + `t` のような促音の表現に使う。
//!
//! 規則表は差し替え可能 ([`RomajiTable::from_rules`])。AZIK などの
//! 別配列は、規則表を差し替えることで対応する。

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
#[derive(Debug, Clone)]
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

    /// 標準の SKK 配列。
    pub fn default_skk() -> Self {
        Self::from_rules(DEFAULT_RULES.iter().map(|&(i, o, n)| Rule::new(i, o, n)))
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

impl Default for RomajiTable {
    fn default() -> Self {
        Self::default_skk()
    }
}

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

        // 2. 撥音: `n` + 子音 → `ん`。(`nn` / `n` + `'` は規則表側で処理済み)
        if chars.len() >= 2 && chars[0] == 'n' && is_consonant(chars[1]) && chars[1] != 'y' {
            let rest: String = chars[1..].iter().collect();
            return Some(("ん".to_owned(), rest));
        }

        // 3. 促音: 同じ子音の連続 → `っ` + 二文字目以降。
        if chars.len() >= 2 && chars[0] == chars[1] && is_consonant(chars[0]) {
            let rest: String = chars[1..].iter().collect();
            return Some(("っ".to_owned(), rest));
        }

        // 4. 一文字だけなら、変換できない文字としてそのまま出す。
        if chars.len() == 1 {
            return Some((buf.to_owned(), String::new()));
        }

        // 5. 先頭を捨てて解釈し直す。
        Some((String::new(), chars[1..].iter().collect()))
    }

    /// 変換を打ち切り、未確定の打鍵列から取り出せるかなを返す。
    ///
    /// 確定操作やモード切り替えの直前に呼ぶ。`n` は `ん` になり、
    /// それ以外の未確定打鍵は捨てられる。
    pub fn flush(&mut self) -> String {
        let pending = std::mem::take(&mut self.pending);
        if pending == "n" {
            "ん".to_owned()
        } else {
            String::new()
        }
    }
}

impl Default for RomajiConverter {
    fn default() -> Self {
        Self::new(RomajiTable::default_skk())
    }
}

fn is_consonant(c: char) -> bool {
    c.is_ascii_alphabetic() && !matches!(c, 'a' | 'i' | 'u' | 'e' | 'o')
}

/// 標準 SKK 配列の変換規則。`(打鍵列, 出力, 持ち越し)`。
#[rustfmt::skip]
const DEFAULT_RULES: &[(&str, &str, &str)] = &[
    ("a", "あ", ""), ("i", "い", ""), ("u", "う", ""), ("e", "え", ""), ("o", "お", ""),

    ("ka", "か", ""), ("ki", "き", ""), ("ku", "く", ""), ("ke", "け", ""), ("ko", "こ", ""),
    ("kya", "きゃ", ""), ("kyi", "きぃ", ""), ("kyu", "きゅ", ""), ("kye", "きぇ", ""), ("kyo", "きょ", ""),
    ("ga", "が", ""), ("gi", "ぎ", ""), ("gu", "ぐ", ""), ("ge", "げ", ""), ("go", "ご", ""),
    ("gya", "ぎゃ", ""), ("gyi", "ぎぃ", ""), ("gyu", "ぎゅ", ""), ("gye", "ぎぇ", ""), ("gyo", "ぎょ", ""),

    ("sa", "さ", ""), ("si", "し", ""), ("su", "す", ""), ("se", "せ", ""), ("so", "そ", ""),
    ("sya", "しゃ", ""), ("syi", "しぃ", ""), ("syu", "しゅ", ""), ("sye", "しぇ", ""), ("syo", "しょ", ""),
    ("sha", "しゃ", ""), ("shi", "し", ""), ("shu", "しゅ", ""), ("she", "しぇ", ""), ("sho", "しょ", ""),
    ("za", "ざ", ""), ("zi", "じ", ""), ("zu", "ず", ""), ("ze", "ぜ", ""), ("zo", "ぞ", ""),
    ("zya", "じゃ", ""), ("zyi", "じぃ", ""), ("zyu", "じゅ", ""), ("zye", "じぇ", ""), ("zyo", "じょ", ""),
    ("ja", "じゃ", ""), ("ji", "じ", ""), ("ju", "じゅ", ""), ("je", "じぇ", ""), ("jo", "じょ", ""),
    ("jya", "じゃ", ""), ("jyi", "じぃ", ""), ("jyu", "じゅ", ""), ("jye", "じぇ", ""), ("jyo", "じょ", ""),

    ("ta", "た", ""), ("ti", "ち", ""), ("tu", "つ", ""), ("te", "て", ""), ("to", "と", ""),
    ("tya", "ちゃ", ""), ("tyi", "ちぃ", ""), ("tyu", "ちゅ", ""), ("tye", "ちぇ", ""), ("tyo", "ちょ", ""),
    ("cha", "ちゃ", ""), ("chi", "ち", ""), ("chu", "ちゅ", ""), ("che", "ちぇ", ""), ("cho", "ちょ", ""),
    ("cya", "ちゃ", ""), ("cyi", "ちぃ", ""), ("cyu", "ちゅ", ""), ("cye", "ちぇ", ""), ("cyo", "ちょ", ""),
    ("tsa", "つぁ", ""), ("tsi", "つぃ", ""), ("tsu", "つ", ""), ("tse", "つぇ", ""), ("tso", "つぉ", ""),
    ("tha", "てゃ", ""), ("thi", "てぃ", ""), ("thu", "てゅ", ""), ("the", "てぇ", ""), ("tho", "てょ", ""),
    ("twa", "とぁ", ""), ("twi", "とぃ", ""), ("twu", "とぅ", ""), ("twe", "とぇ", ""), ("two", "とぉ", ""),
    ("da", "だ", ""), ("di", "ぢ", ""), ("du", "づ", ""), ("de", "で", ""), ("do", "ど", ""),
    ("dya", "ぢゃ", ""), ("dyi", "ぢぃ", ""), ("dyu", "ぢゅ", ""), ("dye", "ぢぇ", ""), ("dyo", "ぢょ", ""),
    ("dha", "でゃ", ""), ("dhi", "でぃ", ""), ("dhu", "でゅ", ""), ("dhe", "でぇ", ""), ("dho", "でょ", ""),
    ("dwa", "どぁ", ""), ("dwi", "どぃ", ""), ("dwu", "どぅ", ""), ("dwe", "どぇ", ""), ("dwo", "どぉ", ""),

    ("na", "な", ""), ("ni", "に", ""), ("nu", "ぬ", ""), ("ne", "ね", ""), ("no", "の", ""),
    ("nya", "にゃ", ""), ("nyi", "にぃ", ""), ("nyu", "にゅ", ""), ("nye", "にぇ", ""), ("nyo", "にょ", ""),
    ("nn", "ん", ""), ("n'", "ん", ""),

    ("ha", "は", ""), ("hi", "ひ", ""), ("hu", "ふ", ""), ("he", "へ", ""), ("ho", "ほ", ""),
    ("hya", "ひゃ", ""), ("hyi", "ひぃ", ""), ("hyu", "ひゅ", ""), ("hye", "ひぇ", ""), ("hyo", "ひょ", ""),
    ("fa", "ふぁ", ""), ("fi", "ふぃ", ""), ("fu", "ふ", ""), ("fe", "ふぇ", ""), ("fo", "ふぉ", ""),
    ("fya", "ふゃ", ""), ("fyu", "ふゅ", ""), ("fyo", "ふょ", ""),
    ("ba", "ば", ""), ("bi", "び", ""), ("bu", "ぶ", ""), ("be", "べ", ""), ("bo", "ぼ", ""),
    ("bya", "びゃ", ""), ("byi", "びぃ", ""), ("byu", "びゅ", ""), ("bye", "びぇ", ""), ("byo", "びょ", ""),
    ("pa", "ぱ", ""), ("pi", "ぴ", ""), ("pu", "ぷ", ""), ("pe", "ぺ", ""), ("po", "ぽ", ""),
    ("pya", "ぴゃ", ""), ("pyi", "ぴぃ", ""), ("pyu", "ぴゅ", ""), ("pye", "ぴぇ", ""), ("pyo", "ぴょ", ""),

    ("ma", "ま", ""), ("mi", "み", ""), ("mu", "む", ""), ("me", "め", ""), ("mo", "も", ""),
    ("mya", "みゃ", ""), ("myi", "みぃ", ""), ("myu", "みゅ", ""), ("mye", "みぇ", ""), ("myo", "みょ", ""),

    ("ya", "や", ""), ("yi", "い", ""), ("yu", "ゆ", ""), ("ye", "いぇ", ""), ("yo", "よ", ""),

    ("ra", "ら", ""), ("ri", "り", ""), ("ru", "る", ""), ("re", "れ", ""), ("ro", "ろ", ""),
    ("rya", "りゃ", ""), ("ryi", "りぃ", ""), ("ryu", "りゅ", ""), ("rye", "りぇ", ""), ("ryo", "りょ", ""),

    ("wa", "わ", ""), ("wi", "うぃ", ""), ("wu", "う", ""), ("we", "うぇ", ""), ("wo", "を", ""),
    ("va", "ヴぁ", ""), ("vi", "ヴぃ", ""), ("vu", "ヴ", ""), ("ve", "ヴぇ", ""), ("vo", "ヴぉ", ""),

    ("xa", "ぁ", ""), ("xi", "ぃ", ""), ("xu", "ぅ", ""), ("xe", "ぇ", ""), ("xo", "ぉ", ""),
    ("xya", "ゃ", ""), ("xyu", "ゅ", ""), ("xyo", "ょ", ""),
    ("xtu", "っ", ""), ("xtsu", "っ", ""), ("xwa", "ゎ", ""),
    ("xka", "ヵ", ""), ("xke", "ヶ", ""), ("xn", "ん", ""),

    ("-", "ー", ""), (",", "、", ""), (".", "。", ""), ("[", "「", ""), ("]", "」", ""),

    // Egg 風の二ストローク記号入力。
    ("z-", "〜", ""), ("z,", "‥", ""), ("z.", "…", ""), ("z/", "・", ""),
    ("z[", "『", ""), ("z]", "』", ""), ("z ", "　", ""),
    ("zh", "←", ""), ("zj", "↓", ""), ("zk", "↑", ""), ("zl", "→", ""),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// 打鍵列を与え、確定したかなを連結して返す。未確定分は捨てる。
    fn typed(keys: &str) -> String {
        let mut c = RomajiConverter::default();
        keys.chars().map(|k| c.feed(k)).collect()
    }

    /// 打鍵列を与え、`(確定したかな, 未確定の打鍵列)` を返す。
    fn typed_with_pending(keys: &str) -> (String, String) {
        let mut c = RomajiConverter::default();
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
    fn flush_resolves_trailing_n_only() {
        let mut c = RomajiConverter::default();
        for k in "hon".chars() {
            c.feed(k);
        }
        assert_eq!(c.flush(), "ん");
        assert!(c.is_empty());

        let mut c = RomajiConverter::default();
        c.feed('k');
        assert_eq!(c.flush(), "");
        assert!(c.is_empty());
    }

    #[test]
    fn backspace_removes_one_keystroke() {
        let mut c = RomajiConverter::default();
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
}
