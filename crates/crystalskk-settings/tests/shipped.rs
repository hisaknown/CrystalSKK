//! 同梱の雛形のままで、ちゃんと動くかを確かめる。
//!
//! 利用者の多くは雛形をそのまま使う。**雛形はそれ自体が一つの設定であり、
//! 試験の外で初めて動かされてよいものではない。** ここでは試験用に手で
//! 書いた値ではなく、`default.toml` と `romaji.txt` を読んだ値でエンジンを
//! 動かす。
//!
//! 一度、雛形の `z/` (・) が効かないまま出荷しかけた。`/` は abbrev を始める
//! キーでもあり、そちらが先に拾っていた。規則を一つずつ打ってみれば
//! 見つかった。

use crystalskk_core::dict::{Candidate, CandidateSource, Query};
use crystalskk_core::engine::Role;
use crystalskk_core::{Engine, InputMode, Key};
use crystalskk_settings::{ROMAJI_TEMPLATE, Settings, TEMPLATE};

fn shipped() -> Settings {
    crystalskk_settings::parse(TEMPLATE, ROMAJI_TEMPLATE).expect("雛形はそのまま読める")
}

/// 見出し語ごとに候補を返し、前方一致で補完する小さな辞書。
struct Dict(Vec<(&'static str, Vec<String>)>);

impl CandidateSource for Dict {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.0
            .iter()
            .filter(|(heading, _)| *heading == query.key)
            .flat_map(|(_, words)| words.iter().map(|w| Candidate::new(w.as_str())))
            .collect()
    }

    fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        self.0
            .iter()
            .map(|(heading, _)| *heading)
            .filter(|key| key.starts_with(prefix) && *key != prefix)
            .take(limit)
            .map(str::to_owned)
            .collect()
    }
}

fn dict() -> Dict {
    Dict(vec![
        ("かんじ", vec!["漢字".to_owned(), "感じ".to_owned()]),
        ("かんじゃ", vec!["患者".to_owned()]),
        ("たくさん", (1..=20).map(|n| format!("候補{n}")).collect()),
    ])
}

/// 雛形の設定を渡したエンジン。ひらがなから始める。
fn configured() -> Engine {
    let mut engine = Engine::new(Box::new(dict()));
    engine.configure(shipped().engine);
    engine
}

fn key_of(c: char) -> Key {
    match c {
        ' ' => Key::Space,
        '\n' => Key::Enter,
        '\t' => Key::Tab,
        c => Key::Char(c),
    }
}

/// 打鍵列を送り、確定した文字列を返す。
fn press_all(engine: &mut Engine, keys: &str) -> String {
    keys.chars()
        .map(|c| engine.press(key_of(c)).commit)
        .collect()
}

#[test]
fn every_rule_in_the_table_can_be_typed() {
    // **表に書いた規則は、打てば出なければならない。** エンジンのキーが
    // 先に拾って、規則が死んでいることがある。
    let settings = shipped();
    let mut dead = Vec::new();
    for rule in settings.engine.romaji.rules() {
        let mut engine = configured();
        let committed = press_all(&mut engine, &rule.input);
        if committed != rule.output {
            dead.push(format!(
                "{:?} → {:?} のはずが {committed:?}",
                rule.input, rule.output
            ));
        }
    }
    assert!(dead.is_empty(), "効かない規則:\n{}", dead.join("\n"));
}

#[test]
fn a_pending_n_does_not_swallow_the_commands() {
    // `n` を打ちかけたまま SKK のキーを打っても、キーはキーとして働く。
    // **規則がキーを横取りすると、`honq` がカタカナにならない。**
    let cases: [(&str, InputMode); 3] = [
        ("honq", InputMode::Katakana),
        ("honl", InputMode::Ascii),
        ("honL", InputMode::FullAscii),
    ];
    for (keys, mode) in cases {
        let mut engine = configured();
        let committed = press_all(&mut engine, keys);
        assert_eq!(committed, "ほん", "{keys}");
        assert_eq!(engine.mode(), mode, "{keys}");
    }

    let mut engine = configured();
    assert_eq!(press_all(&mut engine, "hon "), "ほん ", "空白は空白");

    let mut engine = configured();
    assert_eq!(press_all(&mut engine, "hon/"), "ほん", "n を捨てない");
    assert_eq!(engine.preedit().marker, crystalskk_core::Marker::Composing);
}

#[test]
fn doubled_consonants_and_n_behave_as_usual() {
    let mut engine = configured();
    assert_eq!(press_all(&mut engine, "kitte"), "きって");
    assert_eq!(press_all(&mut engine, "gakkou"), "がっこう");
    // 末尾の `n` は確定するまで打ちかけのまま。Enter で ん になる。
    assert_eq!(press_all(&mut engine, "shinbun\n"), "しんぶん");
    assert_eq!(press_all(&mut engine, "konnnichiha"), "こんにちは");
}

#[test]
fn symbols_after_z_work_while_composing_too() {
    // 見出し語の途中でも同じ。`.` は補完候補を受け取るキーでもあるが、
    // 打ちかけの `z` の続きなら規則が勝つ。
    let mut engine = configured();
    press_all(&mut engine, "Kanz.");
    assert!(engine.preedit().display().starts_with("▽かん…"));
}

#[test]
fn converting_and_listing_work_with_the_shipped_values() {
    let mut engine = configured();
    assert_eq!(press_all(&mut engine, "Kanji \n"), "漢字");

    // 一覧は雛形の回数で出る。ラベルも雛形のもの。
    let settings = shipped();
    let mut engine = configured();
    press_all(&mut engine, "Takusan");
    for _ in 0..settings.engine.candidates.until_list {
        engine.press(Key::Space);
    }
    let view = engine.candidates().expect("選んでいる");
    assert!(view.listing);
    let labels: Vec<char> = view.page().iter().map(|(label, _)| *label).collect();
    assert_eq!(labels, settings.engine.candidates.labels);
}

#[test]
fn dynamic_completion_works_with_the_shipped_values() {
    let settings = shipped();
    let take = settings.engine.completion.take_key;
    let mut engine = configured();
    press_all(&mut engine, "Kann");
    let guess = engine
        .preedit()
        .segments
        .into_iter()
        .find(|s| s.role == Role::Completion)
        .map(|s| s.text);
    assert_eq!(guess.as_deref(), Some("じ"), "二文字で補完候補が出る");
    assert_eq!(press_all(&mut engine, &take.to_string()), "漢字");
}
