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
use crystalskk_core::engine::{PAGE_SIZE, Role, SELECTION_KEYS, UNTIL_CANDIDATE_LIST};
use crystalskk_core::{Engine, Key};

/// 候補をたくさん持つ試験用の辞書。
///
/// 一覧が二ページにまたがる必要があるので、多めに用意する。
struct ManyDict(HashMap<String, Vec<Candidate>>);

/// 候補の数。一覧に載るのは先頭 4 件を除いた分なので、二ページ目まで届く。
const HOW_MANY: usize = 15;

impl ManyDict {
    fn new() -> Self {
        Self::with_candidates(HOW_MANY)
    }

    /// 候補の数を決めて作る。一覧が開く前に尽きる場合を試すのに使う。
    fn with_candidates(how_many: usize) -> Self {
        let words: Vec<Candidate> = (1..=how_many)
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
        Self::with_dict(ManyDict::new())
    }

    fn with_candidates(how_many: usize) -> Self {
        Self::with_dict(ManyDict::with_candidates(how_many))
    }

    fn with_dict(dict: ManyDict) -> Self {
        Self {
            engine: Engine::new(Box::new(dict)),
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
fn escape_returns_to_the_last_page_of_candidates() {
    let mut s = registering();
    s.type_keys("\u{1b}");

    assert!(s.engine.registration().is_none(), "登録を抜ける");
    assert_eq!(s.committed, "", "何も確定しない");

    // 送り切る前に立っていた場所 — 最後のページ — へ返る。
    let view = s.engine.candidates().expect("候補選択に戻っている");
    assert!(view.listing, "一覧が出たままになる");
    assert_eq!(
        view.page().first().map(|(_, c)| c.word.as_str()),
        Some("候補12"),
        "最後のページの先頭"
    );
}

#[test]
fn escape_returns_to_the_last_candidate_when_the_list_never_opened() {
    // 候補が少なく、一覧が開く前に尽きる場合。
    let mut s = Session::with_candidates(2);
    s.convert(3);
    assert!(s.engine.registration().is_some(), "候補を出し切って登録へ");

    s.type_keys("\u{1b}");
    let view = s.engine.candidates().expect("候補選択に戻っている");
    assert!(!view.listing, "一覧は出ていない");
    assert_eq!(view.index, 1, "最後の候補を選んでいる");
    assert_eq!(s.engine.preedit().display(), "▼候補2");
}

#[test]
fn escape_returns_to_the_midashi_when_the_dictionary_had_nothing() {
    // 辞書に一件も無いときは候補選択を経ていない。戻る先は見出し語入力。
    let mut s = Session::with_candidates(0);
    s.convert(1);
    assert!(s.engine.registration().is_some(), "引けずに登録へ");

    s.type_keys("\u{1b}");
    assert!(s.engine.candidates().is_none(), "戻る候補が無い");
    assert_eq!(s.engine.preedit().display(), "▽かんじ");
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

// --- 辞書に届かないとき --------------------------------------------------

/// 引けない辞書。サーバが落ちている場面を写す。
struct Unreachable;

impl CandidateSource for Unreachable {
    fn lookup(&self, _query: &Query) -> Vec<Candidate> {
        Vec::new()
    }

    fn available(&self) -> bool {
        false
    }
}

#[test]
fn an_unreachable_dictionary_does_not_start_a_registration() {
    // **「候補が無い」と「引けなかった」は別である。** 引けなかっただけで
    // 登録を始めると、知っているはずの語を「辞書に無い」と言われたうえ、
    // そのまま登録すれば辞書が汚れる。
    let mut engine = Engine::new(Box::new(Unreachable));
    for key in "Kanji ".chars() {
        engine.press(match key {
            ' ' => Key::Space,
            c => Key::Char(c),
        });
    }

    assert!(engine.registration().is_none(), "登録を始めない");
    assert_eq!(
        engine.preedit().display(),
        "▽かんじ",
        "見出し語入力のまま留まる"
    );
}

#[test]
fn an_empty_but_reachable_dictionary_still_registers() {
    // こちらは本当に無い場合。登録へ進むのが正しい。
    let mut engine = Engine::new(Box::new(ManyDict::with_candidates(0)));
    for key in "Kanji ".chars() {
        engine.press(match key {
            ' ' => Key::Space,
            c => Key::Char(c),
        });
    }
    assert!(engine.registration().is_some(), "辞書に無いなら登録へ");
}

// --- 補完 --------------------------------------------------------------

/// 前方一致を返す辞書。
struct Completing(Vec<String>);

impl CandidateSource for Completing {
    fn lookup(&self, _query: &Query) -> Vec<Candidate> {
        vec![Candidate::new("漢字")]
    }

    fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        self.0
            .iter()
            .filter(|key| key.starts_with(prefix) && key.as_str() != prefix)
            .take(limit)
            .cloned()
            .collect()
    }
}

fn completing() -> Engine {
    Engine::new(Box::new(Completing(vec![
        "かんじ".to_owned(),
        "かんじゃ".to_owned(),
        "かんき".to_owned(),
    ])))
}

/// 打鍵列を送り、未確定の表示を返す。
fn typed(engine: &mut Engine, keys: &str) -> String {
    for c in keys.chars() {
        engine.press(match c {
            ' ' => Key::Space,
            '\t' => Key::Tab,
            c => Key::Char(c),
        });
    }
    engine.preedit().display()
}

#[test]
fn a_guess_appears_once_there_is_enough_to_go_on() {
    let mut engine = completing();
    // 一文字では当てない。**「か」で始まる見出しは山ほどある。**
    assert_eq!(typed(&mut engine, "Ka"), "▽か");
    // 二文字目で当たる。`n` は一つでは確定しないので二度打つ。
    assert_eq!(typed(&mut engine, "nn"), "▽かんじ");
}

#[test]
fn the_guess_is_not_part_of_what_was_typed() {
    let mut engine = completing();
    typed(&mut engine, "Kann");
    let preedit = engine.preedit();
    let typed_text: String = preedit
        .segments
        .iter()
        .filter(|s| s.role != Role::Completion)
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(typed_text, "▽かん", "当て推量は打った文字に含めない");
}

#[test]
fn taking_the_guess_converts_and_commits_it() {
    // **一打鍵で終わる。** 当て推量が出ている時点で見出し語は辞書にあると
    // 分かっているので、変換の結果を選ばせる手間は要らない。
    let mut engine = completing();
    for c in "Kann.".chars() {
        let key = if c == '.' {
            Key::Char('.')
        } else {
            Key::Char(c)
        };
        let response = engine.press(key);
        if !response.commit.is_empty() {
            assert_eq!(response.commit, "漢字");
        }
    }
    assert_eq!(engine.preedit().display(), "", "確定まで進んでいる");
}

#[test]
fn the_window_shows_what_the_dot_would_take() {
    let mut engine = completing();
    typed(&mut engine, "Kann");
    let view = engine.completion().expect("当て推量が出ている");
    assert_eq!(view.heading, "かんじ");
    assert!(!view.taken, "まだ受け取っていない");
}

#[test]
fn the_window_follows_the_tab() {
    let mut engine = completing();
    typed(&mut engine, "Kann		");
    let view = engine.completion().expect("当て推量が出ている");
    assert_eq!(view.heading, "かんじゃ");
    assert!(view.taken, "Tab で当てたものは受け取り済み");
}

#[test]
fn nothing_is_offered_before_there_is_enough_to_go_on() {
    let mut engine = completing();
    typed(&mut engine, "Ka");
    assert!(engine.completion().is_none());
}

#[test]
fn tab_walks_through_the_alternatives() {
    let mut engine = completing();
    assert_eq!(typed(&mut engine, "Kann\t"), "▽かんじ");
    assert_eq!(typed(&mut engine, "\t"), "▽かんじゃ");
    assert_eq!(typed(&mut engine, "\t"), "▽かんき");
    // 端まで来たら先頭へ戻る。
    assert_eq!(typed(&mut engine, "\t"), "▽かんじ");
}

#[test]
fn a_period_is_just_a_period_when_nothing_is_offered() {
    // 当て推量が出ていなければ奪わない。**見出し語に句点も打てる。**
    let mut engine = Engine::new(Box::new(ManyDict::with_candidates(0)));
    let preedit = typed(&mut engine, "Ka.");
    assert!(preedit.ends_with('。'), "普通に句点になる: {preedit}");
}

#[test]
fn converting_ignores_the_guess() {
    // 受け取っていない当て推量は、変換の見出し語に入らない。
    let mut engine = completing();
    typed(&mut engine, "Kann ");
    let view = engine.candidates().expect("変換している");
    assert_eq!(view.candidates[0].word, "漢字");
}
