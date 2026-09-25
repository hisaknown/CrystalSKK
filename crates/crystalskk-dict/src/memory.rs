//! メモリ上に全件を持つ辞書。
//!
//! 送りありと送りなしを別の表に分けて持つ。SKK 辞書自身が二つの区画に
//! 分かれているためであり、また見出しの形だけでは abbrev (`skk`) と
//! 送りあり (`おくr`) を区別できないためでもある。

use std::collections::BTreeMap;

use crystalskk_core::dict::{Candidate, CandidateSource, Query};

use crate::format;

/// 読み込みの結果。壊れた行があっても読み込み自体は成功する。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// 読み込めた見出しの数。
    pub entries: usize,
    /// 解釈できずに読み飛ばした行の数。
    pub skipped: usize,
    /// 既出の見出しと併合した行の数。
    pub merged: usize,
}

/// 全件をメモリに載せた SKK 辞書。
#[derive(Debug, Clone, Default)]
pub struct MemoryDict {
    okuri_ari: BTreeMap<String, Vec<Candidate>>,
    okuri_nashi: BTreeMap<String, Vec<Candidate>>,
    /// 見出しを**新しく使った順**に並べたもの。送りありかどうかを添える。
    ///
    /// **SKK のユーザー辞書は使った順に並んでいる。** 読み書きでこれを
    /// 崩すと、他の SKK から持ち込んだ辞書の順序が失われる。書き出すときは
    /// この順に従う。
    ///
    /// 補完もこの順で出す。辞書順に「かん」で始まる見出しを並べても、
    /// 使う語が先に出るとは限らない。**直前に使った語ほど、また使う。**
    ///
    /// 静的辞書には使った順が無いので空のままになる ([`MemoryDict::parse`]
    /// は順序を覚えない)。17 万件ぶんの見出しを二重に持つ意味がない。
    order: Vec<(bool, String)>,
}

impl MemoryDict {
    pub fn new() -> Self {
        Self::default()
    }

    /// 辞書のテキストを読み込む。
    ///
    /// 区画の注釈行 (`;; okuri-ari entries.`) があればそれに従い、
    /// なければ見出しの形から推測する。
    ///
    /// **並び順は覚えない。** 静的辞書のためのもので、17 万件の見出しを
    /// 二重に持つ意味がない。書き戻す辞書は [`Self::parse_ordered`] で読む。
    pub fn parse(text: &str) -> (Self, LoadReport) {
        Self::read(text, false)
    }

    /// 並び順を覚えながら読み込む。
    ///
    /// ユーザー辞書はこちらで読む。**ファイルの並びがそのまま「使った順」
    /// である** — SKK の辞書はそう書かれる。書き出すときも同じ順に戻す。
    pub fn parse_ordered(text: &str) -> (Self, LoadReport) {
        Self::read(text, true)
    }

    fn read(text: &str, remember_order: bool) -> (Self, LoadReport) {
        let mut dict = Self::new();
        let mut report = LoadReport::default();
        let mut section: Option<bool> = None;

        for line in text.lines() {
            if let Some(is_okuri_ari) = section_marker(line) {
                section = Some(is_okuri_ari);
                continue;
            }
            if line.trim().is_empty() || line.starts_with(';') {
                continue;
            }
            match format::parse_line(line) {
                Some((key, candidates)) => {
                    let okuri_ari = section.unwrap_or_else(|| format::is_okuri_ari_key(&key));
                    if dict.merge(&key, okuri_ari, candidates) {
                        report.merged += 1;
                    } else if remember_order {
                        // 読んだ順が使った順。先頭ほど新しい。
                        dict.order.push((okuri_ari, key.clone()));
                    }
                    report.entries += 1;
                }
                None => report.skipped += 1,
            }
        }
        (dict, report)
    }

    fn table(&self, okuri_ari: bool) -> &BTreeMap<String, Vec<Candidate>> {
        if okuri_ari {
            &self.okuri_ari
        } else {
            &self.okuri_nashi
        }
    }

    fn table_mut(&mut self, okuri_ari: bool) -> &mut BTreeMap<String, Vec<Candidate>> {
        if okuri_ari {
            &mut self.okuri_ari
        } else {
            &mut self.okuri_nashi
        }
    }

    /// 見出しに対する候補。
    pub fn get(&self, key: &str, okuri_ari: bool) -> Option<&[Candidate]> {
        self.table(okuri_ari).get(key).map(Vec::as_slice)
    }

    /// 見出しに候補を足す。既にある語は増やさず、順序は既存のものを優先する。
    ///
    /// 既出の見出しだったなら `true`。配布されている辞書にも同じ見出しが
    /// 二度現れることが実際にあり (SKK-JISYO.L に一件)、後勝ちで上書きすると
    /// 候補を落としてしまう。
    pub fn merge(&mut self, key: &str, okuri_ari: bool, candidates: Vec<Candidate>) -> bool {
        let entry = self.table_mut(okuri_ari).entry(key.to_owned()).or_default();
        let existed = !entry.is_empty();
        for candidate in candidates {
            if !entry.iter().any(|c| c.word == candidate.word) {
                entry.push(candidate);
            }
        }
        existed
    }

    /// 見出しの候補を丸ごと置き換える。
    pub fn insert(&mut self, key: &str, okuri_ari: bool, candidates: Vec<Candidate>) {
        if candidates.is_empty() {
            self.table_mut(okuri_ari).remove(key);
        } else {
            self.table_mut(okuri_ari).insert(key.to_owned(), candidates);
        }
    }

    /// 確定した語を先頭に移す。なければ先頭に加える。
    ///
    /// SKK の学習と辞書登録は、どちらも「この見出しではこの語を最初に出す」
    /// という同じ操作に帰着する。だから両者を分けていない。
    pub fn learn(&mut self, query: &Query, word: &str) {
        self.touch(query.is_okuri_ari(), &query.key);
        let entry = self
            .table_mut(query.is_okuri_ari())
            .entry(query.key.clone())
            .or_default();
        match entry.iter().position(|c| c.word == word) {
            Some(0) => {}
            Some(index) => {
                let candidate = entry.remove(index);
                entry.insert(0, candidate);
            }
            None => entry.insert(0, Candidate::new(word)),
        }
    }

    /// 見出しから候補を一つ消す。候補が無くなれば見出しごと消す。
    /// 消すものがなければ `false`。
    pub fn purge(&mut self, query: &Query, word: &str) -> bool {
        let okuri_ari = query.is_okuri_ari();
        let Some(entry) = self.table_mut(okuri_ari).get_mut(&query.key) else {
            return false;
        };
        let before = entry.len();
        entry.retain(|c| c.word != word);
        if entry.len() == before {
            return false;
        }
        if entry.is_empty() {
            self.remove(&query.key, okuri_ari);
        }
        true
    }

    /// 見出しを一件削除する。消すものがなければ `false`。
    pub fn remove(&mut self, key: &str, okuri_ari: bool) -> bool {
        self.order
            .retain(|(ari, seen)| *ari != okuri_ari || seen != key);
        self.table_mut(okuri_ari).remove(key).is_some()
    }

    /// 見出しを「いま使った」ことにする。先頭へ移す。
    fn touch(&mut self, okuri_ari: bool, key: &str) {
        self.order
            .retain(|(ari, seen)| *ari != okuri_ari || seen != key);
        self.order.insert(0, (okuri_ari, key.to_owned()));
    }

    /// 前方一致する送りなしの見出しを、**使った順**に最大 `limit` 件返す。
    ///
    /// 補完はこれを先に使う (PRD F-11, F-18)。**入力中の見出しそのものも
    /// 返す** — 打ち終えた語が辞書にあるなら、それがいちばん確かな補完で
    /// ある (ADR-0019)。
    ///
    /// 静的辞書には使った順が無いので、何も返らない。そちらは
    /// [`Self::complete`] で辞書順に引く。
    pub fn complete_recent(&self, prefix: &str, limit: usize) -> Vec<&str> {
        if prefix.is_empty() {
            return Vec::new();
        }
        self.order
            .iter()
            .filter(|(okuri_ari, _)| !okuri_ari)
            .map(|(_, key)| key.as_str())
            .filter(|key| key.starts_with(prefix))
            .take(limit)
            .collect()
    }

    /// 前方一致する送りなしの見出しを、辞書順に最大 `limit` 件返す。
    ///
    /// 補完 (PRD F-11, F-18) はこれを使う。**入力中の見出しそのものも返す。**
    /// 辞書順なので、あれば必ず先頭に来る。
    pub fn complete(&self, prefix: &str, limit: usize) -> Vec<&str> {
        if prefix.is_empty() {
            return Vec::new();
        }
        self.okuri_nashi
            .range(prefix.to_owned()..)
            .take_while(|(key, _)| key.starts_with(prefix))
            .take(limit)
            .map(|(key, _)| key.as_str())
            .collect()
    }

    /// 片方の区画を書き出す。
    ///
    /// 使った順を先に、覚えのないものを辞書順で続ける。**順序を知らない
    /// 見出しも落とさない。**
    fn write_section(&self, out: &mut String, okuri_ari: bool) {
        let table = self.table(okuri_ari);
        let mut written: Vec<&str> = Vec::new();

        for (_, key) in self.order.iter().filter(|(ari, _)| *ari == okuri_ari) {
            if let Some(candidates) = table.get(key) {
                out.push_str(&format::format_line(key, candidates));
                out.push('\n');
                written.push(key);
            }
        }
        for (key, candidates) in table {
            if !written.contains(&key.as_str()) {
                out.push_str(&format::format_line(key, candidates));
                out.push('\n');
            }
        }
    }

    /// 収録している見出しの総数。
    pub fn len(&self) -> usize {
        self.okuri_ari.len() + self.okuri_nashi.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// SKK 辞書形式のテキストに書き出す。
    ///
    /// 区画の順序と並び順は SKK の慣例に従う。送りありは降順、送りなしは
    /// 昇順。他の実装が二分探索でこの順序に依存しているため崩さない。
    ///
    /// 比較は UTF-8 のバイト順で行う。慣例の辞書は EUC-JP のバイト順で
    /// 並んでいるため、かな同士の順序は一致するが、ASCII とかなが混ざる
    /// 位置は異なる。読む側は全件を走査するので実害はない。
    pub fn to_skk_text(&self) -> String {
        let mut out = String::new();
        out.push_str(";; -*- mode: fundamental; coding: utf-8 -*-\n");
        // **使った順に書く。** SKK のユーザー辞書は元からこの順で、
        // 読み書きでこれを崩すと、持ち込んだ辞書の順序が失われる。
        out.push_str(";; okuri-ari entries.\n");
        self.write_section(&mut out, true);
        out.push_str(";; okuri-nasi entries.\n");
        self.write_section(&mut out, false);
        out
    }
}

impl CandidateSource for MemoryDict {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.get(&query.key, query.is_okuri_ari())
            .map(<[Candidate]>::to_vec)
            .unwrap_or_default()
    }

    /// 使った順を先に、辞書順を後に。
    ///
    /// 使った覚えのある見出しのほうが、また要る見込みが高い。
    fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        let mut found: Vec<String> = self
            .complete_recent(prefix, limit)
            .into_iter()
            .map(str::to_owned)
            .collect();
        for key in self.complete(prefix, limit) {
            if found.len() >= limit {
                break;
            }
            if !found.iter().any(|seen| seen == key) {
                found.push(key.to_owned());
            }
        }
        found
    }
}

/// 区画を切り替える注釈行なら、それが送りありの区画かを返す。
fn section_marker(line: &str) -> Option<bool> {
    if !line.starts_with(';') {
        return None;
    }
    if line.contains("okuri-ari entries") {
        Some(true)
    } else if line.contains("okuri-nasi entries") || line.contains("okuri-nashi entries") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
;; -*- mode: fundamental; coding: utf-8 -*-
;; okuri-ari entries.
おくr /送/贈/
たべr /食べ/
;; okuri-nasi entries.
かんじ /漢字/感じ/幹事/
かんじゃ /患者/
skk /SKK/
";

    fn sample() -> MemoryDict {
        MemoryDict::parse(SAMPLE).0
    }

    #[test]
    fn parses_both_sections() {
        let (dict, report) = MemoryDict::parse(SAMPLE);
        assert_eq!(report.entries, 5);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.merged, 0);
        assert_eq!(dict.len(), 5);
    }

    #[test]
    fn duplicate_keys_are_merged_not_overwritten() {
        // 配布辞書にも実際にある形。後の行で上書きすると候補が消える。
        let text = ";; okuri-nasi entries.
かんじ /漢字/
かんじ /感じ/漢字/幹事/
";
        let (dict, report) = MemoryDict::parse(text);
        assert_eq!(report.entries, 2);
        assert_eq!(report.merged, 1);
        assert_eq!(dict.len(), 1, "見出しは一つに畳まれる");

        let words: Vec<String> = dict
            .lookup(&Query::okuri_nashi("かんじ"))
            .iter()
            .map(|c| c.word.clone())
            .collect();
        // 先に出てきた候補が先。重複した語は増やさない。
        assert_eq!(words, ["漢字", "感じ", "幹事"]);
    }

    #[test]
    fn section_markers_beat_the_key_shape() {
        let dict = sample();
        // `skk` は末尾が ASCII 英字だが、区画の注釈により送りなしとして読まれる。
        assert!(dict.get("skk", false).is_some());
        assert!(dict.get("skk", true).is_none());
    }

    #[test]
    fn looks_up_by_query() {
        let dict = sample();
        let got = dict.lookup(&Query::okuri_nashi("かんじ"));
        let words: Vec<&str> = got.iter().map(|c| c.word.as_str()).collect();
        assert_eq!(words, ["漢字", "感じ", "幹事"]);

        let got = dict.lookup(&Query::okuri_ari("おく", 'r', "り"));
        assert_eq!(got[0].word, "送");
    }

    #[test]
    fn unknown_keys_give_no_candidates() {
        let dict = sample();
        assert!(dict.lookup(&Query::okuri_nashi("しらない")).is_empty());
    }

    #[test]
    fn broken_lines_are_counted_but_do_not_stop_the_load() {
        let text = "かんじ /漢字/\nこわれた行\nことば /言葉/\n";
        let (dict, report) = MemoryDict::parse(text);
        assert_eq!(report.entries, 2);
        assert_eq!(report.skipped, 1);
        assert!(dict.get("ことば", false).is_some());
    }

    #[test]
    fn learning_moves_a_candidate_to_the_front() {
        let mut dict = sample();
        let query = Query::okuri_nashi("かんじ");
        dict.learn(&query, "幹事");
        let words: Vec<String> = dict.lookup(&query).iter().map(|c| c.word.clone()).collect();
        assert_eq!(words, ["幹事", "漢字", "感じ"]);
    }

    #[test]
    fn purging_drops_one_candidate() {
        let mut dict = sample();
        let query = Query::okuri_nashi("かんじ");
        assert!(dict.purge(&query, "感じ"));
        let words: Vec<String> = dict.lookup(&query).iter().map(|c| c.word.clone()).collect();
        assert_eq!(words, ["漢字", "幹事"]);
        assert!(!dict.purge(&query, "感じ"), "二度目は消すものが無い");
    }

    #[test]
    fn purging_the_last_candidate_drops_the_heading() {
        let mut dict = MemoryDict::new();
        let query = Query::okuri_nashi("みこと");
        dict.learn(&query, "美琴");
        assert!(dict.purge(&query, "美琴"));
        assert!(dict.lookup(&query).is_empty());
        assert!(
            dict.complete_recent("み", 8).is_empty(),
            "使った順からも消える"
        );
    }

    #[test]
    fn learning_an_unknown_word_registers_it() {
        let mut dict = sample();
        let query = Query::okuri_nashi("みこと");
        dict.learn(&query, "尊");
        assert_eq!(dict.lookup(&query)[0].word, "尊");
    }

    #[test]
    fn learning_keeps_okuri_ari_and_nashi_apart() {
        let mut dict = MemoryDict::new();
        dict.learn(&Query::okuri_nashi("skk"), "SKK");
        dict.learn(&Query::okuri_ari("sk", 'k', "き"), "エスケー");

        assert_eq!(dict.get("skk", false).expect("送りなし")[0].word, "SKK");
        assert_eq!(dict.get("skk", true).expect("送りあり")[0].word, "エスケー");
    }

    #[test]
    fn completion_finds_keys_by_prefix() {
        let dict = sample();
        assert_eq!(dict.complete("かん", 10), ["かんじ", "かんじゃ"]);
        // 打ち終えた見出しそのものも返す。辞書順なので先頭に来る。
        assert_eq!(dict.complete("かんじ", 10), ["かんじ", "かんじゃ"]);
        assert!(dict.complete("", 10).is_empty());
        assert!(dict.complete("ない", 10).is_empty());
    }

    #[test]
    fn completion_respects_the_limit() {
        let dict = sample();
        assert_eq!(dict.complete("かん", 1), ["かんじ"]);
    }

    #[test]
    fn completion_ignores_okuri_ari_entries() {
        let dict = sample();
        assert!(dict.complete("おく", 10).is_empty());
    }

    #[test]
    fn writes_back_in_the_order_it_was_read() {
        // **SKK のユーザー辞書は使った順に並んでいる。** 読み書きでこれを
        // 崩すと、他の SKK から持ち込んだ辞書の順序が失われる。
        let text = concat!(
            ";; okuri-ari entries.
",
            "たべr /食べ/
",
            "おくr /送/
",
            ";; okuri-nasi entries.
",
            "かんじゃ /患者/
",
            "skk /SKK/
",
            "かんじ /漢字/
",
        );
        let (dict, _) = MemoryDict::parse_ordered(text);
        let text = dict.to_skk_text();
        let written: Vec<&str> = text.lines().filter(|line| !line.starts_with(';')).collect();
        assert_eq!(
            written,
            vec![
                "たべr /食べ/",
                "おくr /送/",
                "かんじゃ /患者/",
                "skk /SKK/",
                "かんじ /漢字/",
            ]
        );
    }

    #[test]
    fn what_was_just_used_moves_to_the_front() {
        let text = concat!(
            ";; okuri-nasi entries.
",
            "かんじゃ /患者/
",
            "かんじ /漢字/
",
        );
        let (mut dict, _) = MemoryDict::parse_ordered(text);
        dict.learn(&Query::okuri_nashi("かんじ"), "漢字");

        let first = dict
            .to_skk_text()
            .lines()
            .find(|line| !line.starts_with(';'))
            .map(str::to_owned);
        assert_eq!(first.as_deref(), Some("かんじ /漢字/"));
    }

    #[test]
    fn a_dictionary_read_without_order_is_written_in_dictionary_order() {
        // 静的辞書には使った順が無い。落とさずに書ければよい。
        let dict = sample();
        let text = dict.to_skk_text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[1], ";; okuri-ari entries.");
        assert_eq!(lines[2], "おくr /送/贈/");
        assert_eq!(lines[3], "たべr /食べ/");
    }

    #[test]
    fn round_trips_through_text() {
        let dict = sample();
        let (reparsed, report) = MemoryDict::parse(&dict.to_skk_text());
        assert_eq!(report.skipped, 0);
        assert_eq!(reparsed.to_skk_text(), dict.to_skk_text());
    }

    #[test]
    fn removes_entries() {
        let mut dict = sample();
        assert!(dict.remove("かんじ", false));
        assert!(!dict.remove("かんじ", false));
        assert!(dict.lookup(&Query::okuri_nashi("かんじ")).is_empty());
    }
}
