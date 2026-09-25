//! 数値変換。
//!
//! 見出し語の数字を `#` に置き換えて引き、候補の `#0`〜`#9` をその数字で
//! 埋める。`▽1がつ` は `#がつ /#1月/#0月/#3月/` を引き、`１月` `1月` `一月`
//! になる。ddskk の `skk-num` と同じ仕組みで、SKK-JISYO.L もこの形で数値の
//! 見出しを持っている。
//!
//! 型は ddskk に倣う。漢数字の書き方は、ddskk を移した CorvusSKK の
//! `init.lua` に合わせてある。
//!
//! | 型 | 例 (1024) |
//! |---|---|
//! | `#0` | `1024` (そのまま) |
//! | `#1` | `１０２４` (全角) |
//! | `#2` | `一〇二四` (一字ずつ) |
//! | `#3` | `一千二十四` (位取り) |
//! | `#5` | `壱千弐拾四` (大字) |
//! | `#8` | `1,024` (桁区切り) |
//!
//! `#4` (数字で引き直す) と `#9` (将棋の棋譜) は扱わない。その型を含む候補は
//! 出さない。**埋められない候補を `#4` のまま見せても、選びようがない。**

/// 見出し語に含まれる数字の並び。出てきた順。
pub fn numbers(midashi: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current = String::new();
    for c in midashi.chars() {
        if c.is_ascii_digit() {
            current.push(c);
        } else if !current.is_empty() {
            found.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        found.push(current);
    }
    found
}

/// 数字の並びを一つずつ `#` に置き換えた辞書キー。
pub fn key_of(midashi: &str) -> String {
    let mut key = String::new();
    let mut in_number = false;
    for c in midashi.chars() {
        if c.is_ascii_digit() {
            if !in_number {
                key.push('#');
            }
            in_number = true;
        } else {
            key.push(c);
            in_number = false;
        }
    }
    key
}

/// 候補の `#0`〜`#9` を、見出し語の数字で順に埋める。
///
/// 埋められない型があるか、`#` の数が数字より多ければ `None`。
pub fn fill(template: &str, numbers: &[String]) -> Option<String> {
    let mut out = String::new();
    let mut rest = numbers.iter();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        let Some(kind) = chars
            .peek()
            .filter(|_| c == '#')
            .and_then(|d| d.to_digit(10))
        else {
            out.push(c);
            continue;
        };
        chars.next();
        let number = rest.next()?;
        out.push_str(&render(kind, number)?);
    }
    Some(out)
}

/// 登録する語の数字を `#0` に戻す。
///
/// 見出し語が `#` で引かれるので、登録する語も同じ形でなければ次に
/// 引いたときに埋められない。**打った数字の形はそのまま残す** — 全角で
/// 打ったなら `#1`、半角なら `#0`。
pub fn to_template(word: &str) -> String {
    let mut out = String::new();
    let mut kind: Option<char> = None;
    for c in word.chars() {
        let this = if c.is_ascii_digit() {
            Some('0')
        } else if ('０'..='９').contains(&c) {
            Some('1')
        } else {
            None
        };
        if this.is_some() && this != kind {
            out.push('#');
            out.extend(this);
        }
        if this.is_none() {
            out.push(c);
        }
        kind = this;
    }
    out
}

fn render(kind: u32, number: &str) -> Option<String> {
    let digits = || number.chars().filter_map(|c| c.to_digit(10));
    Some(match kind {
        0 => number.to_owned(),
        1 => digits().map(|d| FULLWIDTH[d as usize]).collect(),
        2 => digits().map(|d| KANJI[d as usize]).collect(),
        3 => positional(number, &KANJI, &["", "十", "百", "千"])?,
        5 => positional(number, &DAIJI, &["", "拾", "百", "千"])?,
        8 => grouped(number),
        _ => return None,
    })
}

const FULLWIDTH: [char; 10] = ['０', '１', '２', '３', '４', '５', '６', '７', '８', '９'];
const KANJI: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
const DAIJI: [char; 10] = ['零', '壱', '弐', '参', '四', '五', '六', '七', '八', '九'];

/// 万の位ごとの呼び名。これを越える桁は書けない。
const MYRIADS: [&str; 18] = [
    "",
    "万",
    "億",
    "兆",
    "京",
    "垓",
    "𥝱",
    "穣",
    "溝",
    "澗",
    "正",
    "載",
    "極",
    "恒河沙",
    "阿僧祇",
    "那由他",
    "不可思議",
    "無量大数",
];

/// 位取りの漢数字。十と百の位の「一」は書かない (`十二`、`百五`)。千は
/// 書く (`一千`)。ddskk を移した CorvusSKK と同じ書き方である。
fn positional(number: &str, digits: &[char; 10], units: &[&str; 4]) -> Option<String> {
    let number = number.trim_start_matches('0');
    if number.is_empty() {
        return Some(digits[0].to_string());
    }
    // 頭を 0 で埋めて四桁ずつに揃える。
    let mut values: Vec<usize> = vec![0; (4 - number.len() % 4) % 4];
    values.extend(
        number
            .chars()
            .filter_map(|c| c.to_digit(10))
            .map(|d| d as usize),
    );
    let groups = values.len() / 4;
    if groups > MYRIADS.len() {
        return None;
    }
    let mut out = String::new();
    for (i, group) in values.chunks(4).enumerate() {
        for (j, &d) in group.iter().enumerate() {
            let place = 3 - j;
            if d == 0 {
                continue;
            }
            if d != 1 || place == 0 || place == 3 {
                out.push(digits[d]);
            }
            out.push_str(units[place]);
        }
        if group.iter().any(|&d| d != 0) {
            out.push_str(MYRIADS[groups - 1 - i]);
        }
    }
    Some(out)
}

/// 三桁ごとに `,` で区切る。
fn grouped(number: &str) -> String {
    let len = number.chars().count();
    let mut out = String::new();
    for (i, c) in number.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nums(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn numbers_become_hashes_in_the_key() {
        assert_eq!(key_of("1がつ"), "#がつ");
        assert_eq!(key_of("12じ30ふん"), "#じ#ふん");
        assert_eq!(key_of("かんじ"), "かんじ");
        assert_eq!(numbers("12じ30ふん"), ["12", "30"]);
    }

    #[test]
    fn positional_kanji() {
        let n = |s: &str| fill("#3", &nums(&[s])).unwrap();
        assert_eq!(n("0"), "〇");
        assert_eq!(n("10"), "十");
        assert_eq!(n("12"), "十二");
        assert_eq!(n("105"), "百五");
        assert_eq!(n("1000"), "一千");
        assert_eq!(n("10000"), "一万");
        assert_eq!(n("20030"), "二万三十");
        assert_eq!(n("100000000"), "一億");
    }

    #[test]
    fn other_types() {
        let n = |t: &str, s: &str| fill(t, &nums(&[s]));
        assert_eq!(n("#0", "007").as_deref(), Some("007"));
        assert_eq!(n("#1", "24").as_deref(), Some("２４"));
        assert_eq!(n("#2", "1024").as_deref(), Some("一〇二四"));
        assert_eq!(n("#5", "1024").as_deref(), Some("壱千弐拾四"));
        assert_eq!(n("#8", "1234567").as_deref(), Some("1,234,567"));
        assert_eq!(n("#8", "123").as_deref(), Some("123"));
        assert_eq!(n("#4", "1"), None);
        assert_eq!(n("#9", "34"), None);
    }

    #[test]
    fn a_hash_without_a_type_is_kept() {
        assert_eq!(fill("#は#1", &nums(&["3"])).as_deref(), Some("#は３"));
    }

    #[test]
    fn more_slots_than_numbers_cannot_be_filled() {
        assert_eq!(fill("#0時#0分", &nums(&["3"])), None);
    }

    #[test]
    fn registered_words_turn_back_into_templates() {
        assert_eq!(to_template("1月"), "#0月");
        assert_eq!(to_template("１２月"), "#1月");
        assert_eq!(to_template("3時05分"), "#0時#0分");
        assert_eq!(to_template("月"), "月");
    }
}
