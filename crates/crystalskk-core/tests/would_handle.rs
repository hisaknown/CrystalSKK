//! `would_handle` と `press` の答えが一致することを確かめる。
//!
//! TSF は打鍵を渡す前に「食べるか」を尋ねる。そこで嘘をつくと、打鍵が
//! 消えたり二重に入ったりする。二つの判断は別の場所に書かれているので、
//! 食い違いは試験でしか防げない。
//!
//! 個別の場合を並べるのではなく、あらゆる状態を通しながら毎回照合する。

mod common;

use std::collections::HashMap;

use crystalskk_core::dict::{Candidate, CandidateSource, Query};
use crystalskk_core::{Engine, Key};

struct FixedDict(HashMap<String, Vec<Candidate>>);

impl CandidateSource for FixedDict {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.0.get(&query.key).cloned().unwrap_or_default()
    }
}

fn engine() -> Engine {
    let entries = [
        ("かんじ", &["漢字", "感じ"][..]),
        ("おくr", &["送"][..]),
        ("skk", &["SKK"][..]),
    ];
    let dict = FixedDict(
        entries
            .iter()
            .map(|(key, words)| {
                (
                    (*key).to_owned(),
                    words.iter().map(|w| Candidate::new(*w)).collect(),
                )
            })
            .collect(),
    );
    common::engine(Box::new(dict))
}

/// 照合に使う打鍵。エンジンが分岐する種類を一通り含める。
fn every_key() -> Vec<Key> {
    let mut keys = vec![
        Key::Space,
        Key::Enter,
        Key::Backspace,
        Key::Escape,
        Key::Tab,
        Key::Up,
        Key::Down,
        Key::Ctrl('j'),
        Key::Ctrl('g'),
        Key::Ctrl('q'),
        Key::Ctrl('a'),
    ];
    for c in ['a', 'k', 'n', 'q', 'x', 'l', 'L', 'K', 'A', '/', '1', '-'] {
        keys.push(Key::Char(c));
    }
    keys
}

/// ある状態に持っていくための前置き。
fn preludes() -> Vec<(&'static str, Vec<Key>)> {
    let chars = |s: &str| -> Vec<Key> { s.chars().map(Key::Char).collect() };
    vec![
        ("直接入力", vec![]),
        ("直接入力・ローマ字の途中", chars("k")),
        ("直接入力・撥音の保留", chars("n")),
        ("カタカナ", chars("q")),
        ("半角カタカナ", vec![Key::Ctrl('q')]),
        ("半角英数", chars("l")),
        ("全角英数", chars("L")),
        ("見出し語入力", chars("Kanji")),
        ("見出し語入力・ローマ字の途中", chars("Kanj")),
        ("見出し語入力・送り仮名待ち", chars("OkuR")),
        ("abbrev", chars("/skk")),
        ("候補選択", {
            let mut keys = chars("Kanji");
            keys.push(Key::Space);
            keys
        }),
        ("辞書登録", {
            let mut keys = chars("Mikoto");
            keys.push(Key::Space);
            keys
        }),
        ("辞書登録・入力あり", {
            let mut keys = chars("Mikoto");
            keys.push(Key::Space);
            keys.extend(chars("koto"));
            keys
        }),
        ("辞書登録・ローマ字の途中", {
            let mut keys = chars("Mikoto");
            keys.push(Key::Space);
            keys.extend(chars("k"));
            keys
        }),
        ("辞書登録の中の見出し語入力", {
            let mut keys = chars("Mikoto");
            keys.push(Key::Space);
            keys.extend(chars("Kanji"));
            keys
        }),
    ]
}

#[test]
fn the_two_answers_always_agree() {
    for (name, prelude) in preludes() {
        for key in every_key() {
            let mut engine = engine();
            for k in &prelude {
                engine.press(*k);
            }

            let predicted = engine.would_handle(key);
            let actual = engine.press(key).handled;
            assert_eq!(
                predicted, actual,
                "{name} の状態で {key:?} を渡したとき、\
                 would_handle は {predicted}、press は {actual} と答えた"
            );
        }
    }
}

/// 一打鍵ごとに照合しながら、長い入力を通す。
#[test]
fn the_two_answers_agree_all_the_way_through_a_sentence() {
    let sequences = [
        "Kanji \nno OkuRi\n",
        "kanji ",
        "lhello^J",
        "/skk \n",
        "Mikoto kotoba\n",
        "Kanjiq",
        "KanjixKanji \n",
    ];

    for source in sequences {
        let mut engine = engine();
        let mut keys = source.chars().peekable();
        while let Some(c) = keys.next() {
            let key = match c {
                ' ' => Key::Space,
                '\n' => Key::Enter,
                '^' => match keys.next() {
                    Some(c) => Key::Ctrl(c.to_ascii_lowercase()),
                    None => panic!("^ の後にキーがありません"),
                },
                c => Key::Char(c),
            };

            let predicted = engine.would_handle(key);
            let actual = engine.press(key).handled;
            assert_eq!(predicted, actual, "{source:?} の途中、{key:?} で食い違った");
        }
    }
}
