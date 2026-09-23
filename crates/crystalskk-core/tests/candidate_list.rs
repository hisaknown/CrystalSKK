//! 候補一覧の振る舞いを打鍵列で確かめる試験。
//!
//! SKK の一覧は、他の日本語入力の候補ウィンドウとは見え方が違う。
//!
//! - **最初から出ない。** 何度か送って決まらないとき、初めて開く
//! - **一度に出るのは選べる数だけ。** 選べない候補を並べても仕方がない
//! - **選ぶのはラベルキーを押すこと。** 動く反転カーソルは無い
//!
//! 窓を描く前に、この振る舞いをここで固めておく。**実機でしか確かめられない
//! 部分を、できるだけ小さくしておきたい。**

use std::collections::HashMap;

use crystalskk_core::dict::{Candidate, CandidateSource, Query};
use crystalskk_core::engine::{PAGE_SIZE, SELECTION_KEYS, UNTIL_CANDIDATE_LIST};
use crystalskk_core::{Engine, Key};

/// 候補をたくさん持つ試験用の辞書。
///
/// 一覧が二ページにまたがる必要があるので、多めに用意する。
struct ManyDict(HashMap<String, Vec<Candidate>>);

/// 候補の数。一覧に載るのは先頭 4 件を除いた分なので、二ページ目まで届く。
const HOW_MANY: usize = 15;

impl ManyDict {
    fn new() -> Self {
        let words: Vec<Candidate> = (1..=HOW_MANY)
            .map(|n| Candidate::new(format!("候補{n}")))
            .collect();
        Self(HashMap::from([("かんじ".to_owned(), words)]))
    }
}

impl CandidateSource for ManyDict {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.0.get(&query.key).cloned().unwrap_or_default()
    }
}

struct Session {
    engine: Engine,
    committed: String,
}

impl Session {
    fn new() -> Self {
        Self {
            engine: Engine::new(Box::new(ManyDict::new())),
            committed: String::new(),
        }
    }

    fn type_keys(&mut self, keys: &str) -> &mut Self {
        for c in keys.chars() {
            let key = match c {
                ' ' => Key::Space,
                '\n' => Key::Enter,
                // 取り消し。打鍵列に書けるようにしておく。
                '\u{1b}' => Key::Escape,
                '\u{8}' => Key::Backspace,
                c => Key::Char(c),
            };
            let response = self.engine.press(key);
            self.committed.push_str(&response.commit);
        }
        self
    }

    /// 見出し語を入れ、指定した回数だけ変換する。
    fn convert(&mut self, times: usize) -> &mut Self {
        self.type_keys("Kanji");
        for _ in 0..times {
            self.type_keys(" ");
        }
        self
    }

    fn listing(&self) -> bool {
        self.engine.candidates().expect("候補選択中").listing
    }

    /// いま出ているページの、ラベルと候補の対。
    fn page(&self) -> Vec<(char, String)> {
        self.engine
            .candidates()
            .expect("候補選択中")
            .page()
            .into_iter()
            .map(|(label, candidate)| (label, candidate.word.clone()))
            .collect()
    }
}

#[test]
fn the_list_stays_shut_until_the_fifth_conversion() {
    for times in 1..UNTIL_CANDIDATE_LIST {
        let mut s = Session::new();
        s.convert(times);
        assert!(!s.listing(), "{times} 回目では一覧を出さない");
        assert!(s.page().is_empty(), "{times} 回目では窓に出すものが無い");
    }

    let mut s = Session::new();
    s.convert(UNTIL_CANDIDATE_LIST);
    assert!(s.listing(), "{UNTIL_CANDIDATE_LIST} 回目から一覧を出す");
}

#[test]
fn a_page_holds_exactly_as_many_as_there_are_labels() {
    let mut s = Session::new();
    s.convert(UNTIL_CANDIDATE_LIST);

    let page = s.page();
    assert_eq!(page.len(), PAGE_SIZE, "選べる数より多くは出さない");

    let labels: Vec<char> = page.iter().map(|(label, _)| *label).collect();
    assert_eq!(labels, SELECTION_KEYS.to_vec());

    // 一覧は 5 番目の候補から始まる。それまでは一つずつ見せていた。
    assert_eq!(page[0].1, "候補5");
    assert_eq!(page[PAGE_SIZE - 1].1, "候補11");
}

#[test]
fn a_label_key_commits_that_candidate() {
    let mut s = Session::new();
    s.convert(UNTIL_CANDIDATE_LIST);
    // 三つめのラベル。
    s.type_keys("d");
    assert_eq!(s.committed, "候補7");
}

#[test]
fn space_turns_the_page_instead_of_stepping_one() {
    let mut s = Session::new();
    s.convert(UNTIL_CANDIDATE_LIST);
    s.type_keys(" ");

    let page = s.page();
    assert_eq!(page[0].1, "候補12", "一件ではなく一ページ送る");
    // 残りは 4 件しかないので、そこで切れる。
    assert_eq!(page.len(), HOW_MANY - 11);
}

#[test]
fn a_label_with_no_candidate_does_nothing() {
    let mut s = Session::new();
    s.convert(UNTIL_CANDIDATE_LIST);
    s.type_keys(" ");
    // 二ページ目は 4 件しかない。五つめのラベルは空。
    s.type_keys("k");
    assert_eq!(s.committed, "", "押し間違いで関係のない文字を入れない");
    assert!(s.listing(), "一覧に留まる");
}

#[test]
fn going_back_from_the_first_page_folds_the_list_away() {
    let mut s = Session::new();
    s.convert(UNTIL_CANDIDATE_LIST);
    s.type_keys("x");
    assert!(!s.listing(), "一覧を畳んで一つずつの見え方に返る");

    s.type_keys(" ");
    assert!(s.listing(), "送り直せばまた開く");
}

#[test]
fn going_back_from_a_later_page_returns_to_the_previous_one() {
    let mut s = Session::new();
    s.convert(UNTIL_CANDIDATE_LIST);
    s.type_keys(" ");
    s.type_keys("x");
    assert_eq!(s.page()[0].1, "候補5", "前のページへ戻る");
}

#[test]
fn running_out_of_pages_leads_to_registration() {
    let mut s = Session::new();
    s.convert(UNTIL_CANDIDATE_LIST);
    // 二ページ目まで送り、さらに送ると候補が尽きる。
    s.type_keys("  ");
    assert!(
        s.engine.candidates().is_none(),
        "候補を出し切ったら選択を抜ける"
    );
    assert_eq!(
        s.engine.preedit().registering.as_deref(),
        Some("かんじ"),
        "辞書登録へ進む"
    );
}

// --- 辞書登録 ------------------------------------------------------------

/// 候補を出し切って辞書登録に入ったところまで進める。
fn registering() -> Session {
    let mut s = Session::new();
    s.convert(UNTIL_CANDIDATE_LIST);
    // 二ページ目まで送り、さらに送ると候補が尽きる。
    s.type_keys("  ");
    assert!(s.engine.registration().is_some(), "辞書登録に入っている");
    s
}

#[test]
fn escape_cancels_the_registration_and_returns_to_the_midashi() {
    let mut s = registering();
    s.type_keys("\u{1b}");

    assert!(s.engine.registration().is_none(), "登録を抜ける");
    assert_eq!(
        s.engine.preedit().display(),
        "▽かんじ",
        "見出し語入力に戻る"
    );
    assert_eq!(s.committed, "", "何も確定しない");
}

#[test]
fn ascii_mode_still_feeds_the_registration() {
    let mut s = registering();
    // `l` で半角英数へ移り、そのまま登録語を打つ。
    s.type_keys("labc");

    let view = s.engine.registration().expect("登録中のまま");
    assert_eq!(view.buffer, "abc", "アプリへ抜けずに登録語へ溜まる");
}

#[test]
fn nothing_typed_while_registering_reaches_the_application() {
    let mut s = registering();
    s.type_keys("l");
    // 英数モードでも、登録中はすべて食べる。素通しするとアプリに
    // 文字が入ってしまう。
    for key in [Key::Char('a'), Key::Space, Key::Backspace, Key::Escape] {
        assert!(s.engine.press(key).handled, "{key:?} をアプリへ渡さない");
    }
}

#[test]
fn ascii_mode_registration_can_be_committed() {
    let mut s = registering();
    s.type_keys("labc\n");
    assert_eq!(s.committed, "abc", "登録した語が文書へ入る");
    assert!(s.engine.registration().is_none());
}
