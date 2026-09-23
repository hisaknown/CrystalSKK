//! 変換のときに、周辺情報が候補ソースへ渡ることを確かめる試験。
//!
//! 候補を並べるのはソースの向こう (辞書サーバ) である (ADR-0030)。
//! エンジンが確かめるべきは「変換のときに、周辺情報を添えて引く」ことだけ。

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use crystalskk_core::Key;
use crystalskk_core::dict::{Candidate, CandidateSource, Context, Query};

/// 変換のために引かれたときの周辺情報を覚えておくソース。
#[derive(Default)]
struct Recording {
    seen: Rc<RefCell<Vec<(String, String)>>>,
}

impl CandidateSource for Recording {
    fn lookup(&self, _query: &Query) -> Vec<Candidate> {
        vec![Candidate::new("漢字"), Candidate::new("幹事")]
    }

    fn lookup_for_conversion(&self, query: &Query, context: &Context) -> Vec<Candidate> {
        self.seen
            .borrow_mut()
            .push((context.text_before(100), context.text_after(5)));
        self.lookup(query)
    }
}

fn type_keys(engine: &mut crystalskk_core::Engine, keys: &str) -> String {
    let mut committed = String::new();
    for c in keys.chars() {
        let key = match c {
            ' ' => Key::Space,
            '\n' => Key::Enter,
            c => Key::Char(c),
        };
        committed.push_str(&engine.press(key).commit);
    }
    committed
}

#[test]
fn the_surroundings_go_with_a_conversion() {
    let source = Recording::default();
    let seen = source.seen.clone();
    let mut engine = common::engine(Box::new(source));
    engine.set_surroundings(Some("会議の".to_owned()), Some("を務めた。以後".to_owned()));

    type_keys(&mut engine, "Kanji ");

    assert_eq!(
        *seen.borrow(),
        [("会議の".to_owned(), "を務めた。".to_owned())]
    );
}

#[test]
fn without_the_text_before_what_was_committed_stands_in() {
    let source = Recording::default();
    let seen = source.seen.clone();
    let mut engine = common::engine(Box::new(source));

    // 周辺テキストを返さないアプリ。確定した文字列だけが分かっている。
    type_keys(&mut engine, "kaiginoKanji ");

    assert_eq!(*seen.borrow(), [("かいぎの".to_owned(), String::new())]);
}
