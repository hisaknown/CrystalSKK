//! 辞書から辞書を作る。
//!
//! SKK にはカタカナ語の辞書が無い。L 辞書にはカタカナ語が大量に載って
//! いるが、**自分の読みの見出しの下に無いことが多い**。`ヴァイオリン` は
//! `violin` (abbrev 用の英字の見出し) の下にしか無く、`う゛ぁいおりん` と
//! 打っても出てこない。
//!
//! そこで、辞書に載っているカタカナ語を拾い、**その語自身の読み**を見出しに
//! した辞書を作る (ADR-0023)。規則は SKKFEP に添えられていたスクリプト
//! (`skkdict_kana.js`) に合わせてある。

use std::collections::BTreeMap;

/// 辞書の本文から、カタカナ語の辞書の本文を作る。
///
/// - 候補の語 (`;` より前) が 2 文字以上で、すべて `ァ`〜`ー` の範囲に
///   あるものを拾う。送りあり・送りなしは問わない。
/// - 注釈も含めて `?` のある候補は拾わない。辞書で「自信が無い」とされた
///   ものである。
/// - 見出しはその語から作る。`ァ`〜`ン` はひらがなに、`ヴ` は `う゛` にし、
///   `ー` `・` `ヵ` `ヶ` などはそのまま。読みが語と変わらないもの (`ー`
///   だけ、など) は拾わない。
/// - 同じ見出しには候補を一つだけ置く。同じ語が注釈違いで何度も出てくる
///   ときは、長いほう (注釈の詳しいほう) を残す。
pub fn katakana_words(text: &str) -> String {
    let mut found: BTreeMap<String, &str> = BTreeMap::new();

    for line in text.lines() {
        if line.starts_with(';') {
            continue;
        }
        let fields: Vec<&str> = line.split('/').collect();
        // 最初は見出し、最後は行末の空。その間が候補。
        for candidate in fields.iter().take(fields.len().saturating_sub(1)).skip(1) {
            if candidate.contains('?') {
                continue;
            }
            let word = candidate.split(';').next().unwrap_or_default();
            let Some(reading) = reading_of(word) else {
                continue;
            };
            let longer = found
                .get(&reading)
                .is_none_or(|kept| kept.chars().count() < candidate.chars().count());
            if longer {
                found.insert(reading, candidate);
            }
        }
    }

    let mut out = String::from(";; okuri-nasi entries.\n");
    for (reading, candidate) in found {
        out.push_str(&format!("{reading} /{candidate}/\n"));
    }
    out
}

/// カタカナ語の読み。カタカナ語でなければ `None`。
fn reading_of(word: &str) -> Option<String> {
    if word.chars().count() < 2 {
        return None;
    }
    let mut reading = String::with_capacity(word.len());
    for c in word.chars() {
        match c {
            'ァ'..='ン' => reading.push(char::from_u32(c as u32 - 0x60)?),
            'ヴ' => reading.push_str("う゛"),
            'ヵ'..='ー' => reading.push(c),
            _ => return None,
        }
    }
    (reading != word).then_some(reading)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<&str> {
        text.lines().filter(|l| !l.starts_with(';')).collect()
    }

    #[test]
    fn a_word_hidden_under_an_english_heading_gets_its_own_reading() {
        // L 辞書では ヴァイオリン は violin の下にしか無い。
        let derived = katakana_words("violin /ヴァイオリン/バイオリン/\n");
        assert_eq!(
            lines(&derived),
            ["う゛ぁいおりん /ヴァイオリン/", "ばいおりん /バイオリン/"]
        );
    }

    #[test]
    fn only_katakana_words_are_picked() {
        let derived = katakana_words(
            ";; okuri-nasi entries.\nかんじ /漢字/カンジ/感じ/\nぱそこん /パソコン;personal computer/\nあ /ア/\n",
        );
        // 漢字は拾わない。一文字の語も拾わない。注釈は残す。
        assert_eq!(
            lines(&derived),
            ["かんじ /カンジ/", "ぱそこん /パソコン;personal computer/"]
        );
    }

    #[test]
    fn doubtful_candidates_are_left_out() {
        let derived = katakana_words("てすと /テスト;?/テストパターン/\n");
        assert_eq!(lines(&derived), ["てすとぱたーん /テストパターン/"]);
    }

    #[test]
    fn marks_stay_as_they_are() {
        let derived = katakana_words("x /ジョン・スミス/ヵ月/ーー/\n");
        // ・ と ー は読みにもそのまま入る。漢字の混じる語は拾わない。
        // ー だけの語は読みが語と変わらないので拾わない。
        assert_eq!(lines(&derived), ["じょん・すみす /ジョン・スミス/"]);
    }

    #[test]
    fn one_candidate_per_reading_the_most_detailed() {
        let derived = katakana_words("a /コンピュータ/\nb /コンピュータ;computer/\n");
        assert_eq!(lines(&derived), ["こんぴゅーた /コンピュータ;computer/"]);
    }

    #[test]
    fn comments_are_not_read() {
        assert!(lines(&katakana_words(";; /コメント/\n")).is_empty());
    }
}
