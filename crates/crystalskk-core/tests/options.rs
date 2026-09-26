//! 設定のとおりに振る舞うかを確かめる。
//!
//! エンジンは既定値を持たない (ADR-0020)。**受け取るまでは何もせず、
//! 受け取ったら書かれたとおりに動く。** その両方をここで見る。

mod common;

use common::rebind;
use crystalskk_core::dict::{Candidate, CandidateSource, Query};
use crystalskk_core::engine::Role;
use crystalskk_core::{Command, Engine, Key};

/// 見出し語ごとに候補を返し、前方一致で補完する。
struct Dict(Vec<(&'static str, Vec<String>)>);

impl Dict {
    fn sample() -> Self {
        Self(vec![
            ("かん", vec!["巻".to_owned()]),
            ("かんじ", vec!["漢字".to_owned()]),
            ("かんじゃ", vec!["患者".to_owned()]),
            ("たくさん", (1..=20).map(|n| format!("候補{n}")).collect()),
        ])
    }
}

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
            .filter(|key| key.starts_with(prefix))
            .take(limit)
            .map(str::to_owned)
            .collect()
    }
}

fn press_all(engine: &mut Engine, keys: &str) -> String {
    let mut committed = String::new();
    for c in keys.chars() {
        let key = match c {
            ' ' => Key::Space,
            '\t' => Key::Tab,
            '\n' => Key::Enter,
            c => Key::Char(c),
        };
        committed.push_str(&engine.press(key).commit);
    }
    committed
}

fn ghost(engine: &Engine) -> Option<String> {
    engine
        .preedit()
        .segments
        .into_iter()
        .find(|s| s.role == Role::Completion)
        .map(|s| s.text)
}

#[test]
fn without_settings_every_key_goes_to_the_application() {
    // **既定の値で動き出さない。** 動けば、利用者のファイルに書かれて
    // いない値が効くことになる。
    let mut engine = Engine::new(Box::new(Dict::sample()));
    for key in [Key::Char('K'), Key::Char('a'), Key::Space, Key::Ctrl('j')] {
        assert!(!engine.would_handle(key), "{key:?} を食べない");
        let response = engine.press(key);
        assert!(!response.handled, "{key:?} を食べない");
        assert!(response.commit.is_empty());
    }
    assert!(engine.preedit().is_empty());
}

#[test]
fn settings_arrive_later_and_take_effect() {
    let mut engine = Engine::new(Box::new(Dict::sample()));
    assert!(!engine.is_configured());
    engine.configure(common::options());
    assert!(engine.is_configured());
    assert_eq!(press_all(&mut engine, "Kanji \n"), "漢字");
}

#[test]
fn turning_dynamic_completion_off_hides_the_guess() {
    let mut options = common::options();
    options.completion.dynamic = false;
    let mut engine = Engine::new(Box::new(Dict::sample()));
    engine.configure(options);

    press_all(&mut engine, "Kann");
    assert_eq!(ghost(&engine), None, "打っている最中には補完しない");
    assert!(engine.completion().is_none());
    // `.` はただの句点になる。
    press_all(&mut engine, ".");
    assert!(engine.preedit().display().ends_with('。'));
}

#[test]
fn tab_still_completes_when_dynamic_completion_is_off() {
    // Tab は利用者が呼ぶもの。動的補完を切っても使える。
    let mut options = common::options();
    options.completion.dynamic = false;
    let mut engine = Engine::new(Box::new(Dict::sample()));
    engine.configure(options);

    press_all(&mut engine, "Kann");
    assert!(
        engine.would_handle(Key::Tab),
        "引いてみて候補があるなら食べる"
    );
    press_all(&mut engine, "\t");
    // 打った見出し (かん) が辞書にあるので、まずそれを選ぶ。
    assert_eq!(engine.preedit().display(), "▽かん");
    let view = engine.completion().expect("巡っている");
    assert!(view.taken);
    press_all(&mut engine, "\t");
    assert_eq!(engine.preedit().display(), "▽かんじ");
}

#[test]
fn the_minimum_length_decides_when_guessing_starts() {
    let mut options = common::options();
    options.completion.min_length = 3;
    let mut engine = Engine::new(Box::new(Dict::sample()));
    engine.configure(options);

    press_all(&mut engine, "Kann");
    assert!(engine.completion().is_none(), "二文字ではまだ補完しない");
    press_all(&mut engine, "ji");
    let view = engine.completion().expect("三文字で補完する");
    assert_eq!(view.current().word, "漢字");
}

#[test]
fn the_take_key_can_be_changed() {
    let mut options = common::options();
    options.keys = rebind(Command::TakeCompletion, &[Key::Char(',')]);
    let mut engine = Engine::new(Box::new(Dict::sample()));
    engine.configure(options);

    assert_eq!(press_all(&mut engine, "Kanji,"), "漢字");
}

#[test]
fn the_labels_decide_the_page() {
    let mut options = common::options();
    options.candidates.labels = vec!['1', '2', '3'];
    options.candidates.until_list = 2;
    let mut engine = Engine::new(Box::new(Dict::sample()));
    engine.configure(options);

    press_all(&mut engine, "Takusan  ");
    let view = engine.candidates().expect("選んでいる");
    assert!(view.listing, "二回目の変換から一覧に移る");
    let labels: Vec<char> = view.page().iter().map(|(label, _)| *label).collect();
    assert_eq!(labels, ['1', '2', '3']);
    assert_eq!(press_all(&mut engine, "2"), "候補3");
}

#[test]
fn changing_the_settings_drops_what_was_in_progress() {
    // 区切り方が変われば、選んでいる途中の一覧はもう同じ形をしていない。
    let mut engine = common::engine(Box::new(Dict::sample()));
    press_all(&mut engine, "Kanji ");
    assert!(engine.candidates().is_some());

    let mut options = common::options();
    options.candidates.labels = vec!['1', '2'];
    engine.configure(options);
    assert!(engine.candidates().is_none());
    assert!(engine.preedit().is_empty());
}

#[test]
fn the_same_settings_again_change_nothing() {
    // 設定は入力先が変わるたびに渡し直される。**同じものなら、打ちかけを
    // 捨てない。**
    let mut engine = common::engine(Box::new(Dict::sample()));
    press_all(&mut engine, "Kanji");
    engine.configure(common::options());
    assert!(engine.preedit().display().starts_with("▽かんじ"));
}

fn engine_with(keys: crystalskk_core::Keymap) -> Engine {
    let mut options = common::options();
    options.keys = keys;
    let mut engine = Engine::new(Box::new(Dict::sample()));
    engine.configure(options);
    engine
}

#[test]
fn the_keys_follow_the_settings() {
    // `Ctrl+J` の代わりに `Ctrl+M` で確定し、かなへ戻る。
    let mut engine = engine_with(rebind(Command::Hiragana, &[Key::Ctrl('m')]));
    press_all(&mut engine, "l");
    assert!(!engine.press(Key::Ctrl('j')).handled, "外したキーは素通し");
    engine.press(Key::Ctrl('m'));
    assert_eq!(press_all(&mut engine, "ka"), "か");
}

#[test]
fn a_command_can_have_several_keys() {
    let mut engine = engine_with(rebind(
        Command::PreviousCandidate,
        &[Key::Char('x'), Key::Ctrl('p')],
    ));
    press_all(&mut engine, "Takusan  ");
    engine.press(Key::Ctrl('p'));
    let view = engine.candidates().expect("選んでいる");
    assert_eq!(view.index, 0);
}

#[test]
fn a_command_without_keys_is_not_there() {
    // `l` に何も割り当てなければ、ただの文字として打てる。
    let mut engine = engine_with(rebind(Command::Ascii, &[]));
    press_all(&mut engine, "lo");
    assert_eq!(engine.mode(), crystalskk_core::InputMode::Hiragana);
}

#[test]
fn a_space_that_does_not_convert_is_part_of_the_reading() {
    let mut engine = engine_with(rebind(Command::StartHenkan, &[Key::Ctrl('t')]));
    press_all(&mut engine, "/a b");
    assert!(engine.candidates().is_none(), "空白では変換しない");
    engine.press(Key::Ctrl('t'));
    assert!(engine.candidates().is_some() || engine.registration().is_some());
}

#[test]
fn a_command_that_does_nothing_goes_to_the_application() {
    // 取り消すものも確定するものも無ければ、キーはアプリのもの。
    let mut engine = engine_with(common::keymap());
    for key in [Key::Ctrl('g'), Key::Escape, Key::Enter, Key::Backspace] {
        assert!(!engine.would_handle(key), "{key:?}");
        assert!(!engine.press(key).handled, "{key:?}");
    }
}

#[test]
fn a_command_that_undoes_typing_is_eaten() {
    let mut engine = engine_with(common::keymap());
    press_all(&mut engine, "k");
    assert!(engine.press(Key::Ctrl('g')).handled, "打ちかけを捨てる");
    assert_eq!(press_all(&mut engine, "a"), "あ", "k は捨てられている");
}

#[test]
fn enter_confirms_without_changing_the_mode() {
    // Enter (kakutei) は Ctrl+J (hiragana) と違い、英数からかなへ戻さない。
    let mut engine = engine_with(common::keymap());
    press_all(&mut engine, "l");
    assert!(
        !engine.press(Key::Enter).handled,
        "英数の Enter は改行のまま"
    );
    assert_eq!(engine.mode(), crystalskk_core::InputMode::Ascii);
    engine.press(Key::Ctrl('j'));
    assert_eq!(engine.mode(), crystalskk_core::InputMode::Hiragana);
}

#[test]
fn arrows_are_not_skk_keys() {
    let mut engine = engine_with(common::keymap());
    press_all(&mut engine, "Takusan ");
    assert!(!engine.press(Key::Down).handled);
    assert!(engine.candidates().is_some(), "候補はそのまま");
}

#[test]
fn going_back_to_hiragana_is_eaten_even_in_hiragana() {
    // ひらがなへ戻す操作は冪等。すでにひらがなでもアプリへ漏らさない。
    let mut engine = engine_with(common::keymap());
    assert!(engine.would_handle(Key::Ctrl('j')));
    assert!(engine.press(Key::Ctrl('j')).handled);
    assert_eq!(engine.mode(), crystalskk_core::InputMode::Hiragana);
}

#[test]
fn kakutei_newline_confirms_and_lets_the_key_through() {
    let mut engine = engine_with(rebind(Command::KakuteiNewline, &[Key::Ctrl('m')]));
    press_all(&mut engine, "Kanji");
    let response = engine.press(Key::Ctrl('m'));
    assert_eq!(response.commit, "かんじ");
    assert!(
        response.handled && response.pass_through,
        "確定して、キーも渡す"
    );

    press_all(&mut engine, "Kanji ");
    let response = engine.press(Key::Ctrl('m'));
    assert_eq!(response.commit, "漢字");
    assert!(response.pass_through);

    // 確定するものが無ければ、ただ素通しする。
    let response = engine.press(Key::Ctrl('m'));
    assert!(!response.handled && !response.pass_through);
}

#[test]
fn kakutei_newline_does_not_leave_the_registration() {
    // 登録の中で確定した語は欄に入る。キーを文書へ渡してはいけない。
    let mut engine = engine_with(rebind(Command::KakuteiNewline, &[Key::Ctrl('m')]));
    press_all(&mut engine, "Mikoto ");
    assert!(engine.registration().is_some(), "登録に入った");
    press_all(&mut engine, "Kanji");
    let response = engine.press(Key::Ctrl('m'));
    assert!(!response.pass_through);
    assert!(engine.registration().is_some(), "まだ登録の中");
}
