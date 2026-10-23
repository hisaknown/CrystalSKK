//! ネットの候補 (ADR-0042) を打鍵列で確かめる試験。
//!
//! 尋ねるのは辞書の候補が尽きたときだけで、答えは待たない。取りに行って
//! いるあいだは留まり、もう一度送れば尋ね直す。

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crystalskk_core::dict::{Candidate, CandidateSource, Query, Web};
use crystalskk_core::{Engine, Key};

/// ネットの側の様子。試験から書き換える。
#[derive(Default)]
struct Remote {
    answer: RefCell<Option<Web>>,
    asked: Cell<usize>,
}

/// 「かんじ」に決まった数の候補を持ち、ネットの答えを `Remote` に任せる辞書。
struct WebDict {
    local: Vec<Candidate>,
    remote: Rc<Remote>,
}

impl CandidateSource for WebDict {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        if query.key == "かんじ" {
            self.local.clone()
        } else {
            Vec::new()
        }
    }

    fn lookup_web(&self, _query: &Query) -> Web {
        self.remote.asked.set(self.remote.asked.get() + 1);
        self.remote.answer.borrow().clone().unwrap_or(Web::Off)
    }
}

struct Session {
    engine: Engine,
    remote: Rc<Remote>,
}

impl Session {
    fn new(local: &[&str], answer: Web) -> Self {
        let remote = Rc::new(Remote::default());
        *remote.answer.borrow_mut() = Some(answer);
        let dict = WebDict {
            local: local.iter().map(|w| Candidate::new(*w)).collect(),
            remote: Rc::clone(&remote),
        };
        Self {
            engine: common::engine(Box::new(dict)),
            remote,
        }
    }

    fn answer(&self, answer: Web) {
        *self.remote.answer.borrow_mut() = Some(answer);
    }

    fn type_keys(&mut self, keys: &str) -> &mut Self {
        for c in keys.chars() {
            let key = match c {
                ' ' => Key::Space,
                c => Key::Char(c),
            };
            self.engine.press(key);
        }
        self
    }

    fn words(&self) -> Vec<String> {
        self.engine
            .candidates()
            .expect("候補選択中")
            .candidates
            .iter()
            .map(|c| c.word.clone())
            .collect()
    }

    fn registering(&self) -> bool {
        self.engine.registration().is_some()
    }
}

fn found(words: &[&str]) -> Web {
    Web::Found(words.iter().map(|w| Candidate::new(*w)).collect())
}

#[test]
fn the_web_is_not_asked_while_the_dictionary_has_more() {
    let mut s = Session::new(&["漢字", "感じ", "幹事"], found(&["かんじ"]));
    s.type_keys("Kanji ");
    assert_eq!(s.remote.asked.get(), 0, "ふだんの変換では尋ねない");
}

#[test]
fn the_last_candidate_asks_ahead() {
    let mut s = Session::new(&["漢字", "感じ"], found(&["寛治"]));
    s.type_keys("Kanji  ");
    assert_eq!(s.remote.asked.get(), 1, "最後の一件で先に尋ねる");
    assert_eq!(s.words(), ["漢字", "感じ", "寛治"], "辞書の後ろに付け足す");

    s.type_keys(" ");
    assert_eq!(s.engine.preedit().display(), "▼寛治");
}

#[test]
fn words_the_dictionary_has_are_not_repeated() {
    let mut s = Session::new(&["漢字"], found(&["漢字", "寛治"]));
    s.type_keys("Kanji");
    s.type_keys(" ");
    assert_eq!(s.words(), ["漢字", "寛治"]);
}

#[test]
fn a_pending_answer_holds_the_last_candidate() {
    let mut s = Session::new(&["漢字"], Web::Pending);
    s.type_keys("Kanji ");
    assert!(s.engine.web_pending(), "取りに行っていると知らせる");

    s.type_keys(" ");
    assert!(!s.registering(), "届くまでは辞書登録へ進まない");
    assert_eq!(s.engine.preedit().display(), "▼漢字");
    assert!(s.engine.web_pending());
    assert!(s.engine.poll_web().is_none(), "まだなら見え方は変わらない");
    assert!(s.engine.web_pending());

    s.answer(found(&["寛治"]));
    let response = s.engine.poll_web().expect("届いた");
    assert!(!s.engine.web_pending());
    assert_eq!(
        response.preedit.display(),
        "▼寛治",
        "届いたら、待たせた Space の続きで送る"
    );
}

#[test]
fn an_answer_asked_ahead_is_appended_without_moving() {
    let mut s = Session::new(&["漢字"], Web::Pending);
    s.type_keys("Kanji ");
    s.answer(found(&["寛治"]));
    let response = s.engine.poll_web().expect("届いた");
    assert_eq!(response.preedit.display(), "▼漢字", "送ってはいない");
    assert_eq!(s.words(), ["漢字", "寛治"]);
}

#[test]
fn nothing_arriving_after_a_wait_leads_to_registration() {
    let mut s = Session::new(&["漢字"], Web::Pending);
    s.type_keys("Kanji  ");
    s.answer(found(&[]));
    s.engine.poll_web().expect("届いた");
    assert!(s.registering(), "待たせた Space は辞書登録へ進むはずだった");
}

#[test]
fn a_key_while_waiting_forgets_the_wait() {
    let mut s = Session::new(&[], Web::Pending);
    s.type_keys("Kurisutaru ");
    s.engine.press(Key::Escape);
    s.answer(found(&["クリスタル"]));
    assert!(!s.engine.web_pending());
    assert!(s.engine.poll_web().is_none(), "取り消したものは変換しない");
}

#[test]
fn nothing_found_leads_to_registration() {
    let mut s = Session::new(&["漢字"], found(&[]));
    s.type_keys("Kanji ");
    s.type_keys(" ");
    assert!(s.registering());
    assert_eq!(s.remote.asked.get(), 1, "「無い」と言われたらもう尋ねない");
}

#[test]
fn off_leads_to_registration_as_before() {
    let mut s = Session::new(&["漢字"], Web::Off);
    s.type_keys("Kanji  ");
    assert!(s.registering());
}

#[test]
fn an_empty_dictionary_asks_the_web() {
    let mut s = Session::new(&[], Web::Pending);
    s.type_keys("Kurisutaru ");
    assert!(!s.registering(), "取りに行っているあいだは登録しない");
    assert_eq!(s.engine.preedit().display(), "▽くりすたる");
    assert!(s.engine.web_pending());

    s.answer(found(&["クリスタル"]));
    let response = s.engine.poll_web().expect("届いた");
    assert_eq!(
        response.preedit.display(),
        "▼クリスタル",
        "届いたら変換して見せる"
    );
}

#[test]
fn an_empty_dictionary_with_nothing_on_the_web_registers() {
    let mut s = Session::new(&[], found(&[]));
    s.type_keys("Kurisutaru ");
    assert!(s.registering());
}

#[test]
fn choosing_a_web_candidate_is_learned() {
    let mut s = Session::new(&[], found(&["クリスタル"]));
    s.type_keys("Kurisutaru ");
    let response = s.engine.press(Key::Enter);
    assert_eq!(response.commit, "クリスタル");
    assert!(
        response.events.iter().any(|e| matches!(
            e,
            crystalskk_core::engine::Event::Learn { word, .. } if word == "クリスタル"
        )),
        "選べばユーザー辞書に入る"
    );
}
