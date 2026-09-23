//! 辞書と繋いだセッションの試験。
//!
//! エンジン単体の試験 (`crystalskk-core`) と違い、こちらは辞書の読み込み・
//! 学習・保存まで含めた通し動作を見る。

use std::fs;
use std::path::PathBuf;

use crystalskk_cli::keys;
use crystalskk_cli::session::{Session, SessionBuilder};

const DICTIONARY: &str = "\
;; -*- coding: utf-8 -*-
;; okuri-ari entries.
おくr /送/贈/
;; okuri-nasi entries.
かんじ /漢字/感じ;feeling/幹事/
";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("crystalskk-cli-test-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("作業ディレクトリを作れる");
    dir
}

/// 辞書とユーザー辞書を用意したセッション。
fn session(name: &str) -> (Session, PathBuf) {
    let dir = scratch(name);
    let dictionary = dir.join("SKK-JISYO.test");
    fs::write(&dictionary, DICTIONARY).expect("辞書を置ける");
    let user = dir.join("user.dict");

    let mut log = |_: &str| {};
    let session = SessionBuilder::default()
        .dictionary(&dictionary)
        .user_dictionary(&user)
        .settings(dir.join("config.toml"))
        .build(&mut log)
        .expect("セッションを作れる");
    (session, user)
}

/// 打鍵列の表記で入力する。
fn type_keys(session: &mut Session, source: &str) {
    session.press_all(keys::parse(source).expect("読める表記"));
}

#[test]
fn converts_with_the_loaded_dictionary() {
    let (mut session, _) = session("convert");
    type_keys(&mut session, "Kanji\\s");
    assert_eq!(session.preedit(), "▼漢字");

    type_keys(&mut session, "\\n");
    assert_eq!(session.document(), "漢字");
}

#[test]
fn shows_annotations_from_the_dictionary() {
    let (mut session, _) = session("annotation");
    type_keys(&mut session, "Kanji\\s\\s");
    let view = session.candidates().expect("候補選択中");
    assert_eq!(view.candidates[view.index].word, "感じ");
    assert_eq!(
        view.candidates[view.index].annotation.as_deref(),
        Some("feeling")
    );
}

#[test]
fn learning_reorders_the_next_conversion() {
    let (mut session, _) = session("learn");
    // 三番目の候補を選んで確定する。
    type_keys(&mut session, "Kanji\\s\\s\\s\\n");
    assert_eq!(session.document(), "幹事");

    // 次からは選んだものが先頭に来る。
    type_keys(&mut session, "Kanji\\s");
    assert_eq!(session.preedit(), "▼幹事");
}

#[test]
fn registration_writes_to_the_user_dictionary() {
    let (mut session, path) = session("register");
    // 辞書にない見出しは登録に入る。
    type_keys(&mut session, "Mikoto\\s");
    assert_eq!(session.registration_depth(), 1);
    assert_eq!(session.registering().as_deref(), Some("みこと"));

    type_keys(&mut session, "kotoba\\n");
    assert_eq!(session.document(), "ことば");
    assert_eq!(session.registration_depth(), 0);

    assert!(session.save_user_dictionary().expect("保存できる"));
    let saved = fs::read_to_string(&path).expect("読める");
    assert!(saved.contains("みこと /ことば/"), "実際の中身: {saved}");
}

#[test]
fn okuri_conversion_goes_through_the_dictionary() {
    let (mut session, _) = session("okuri");
    type_keys(&mut session, "OkuRi");
    assert_eq!(session.preedit(), "▼送り");
    type_keys(&mut session, "\\n");
    assert_eq!(session.document(), "送り");
}

#[test]
fn unhandled_enter_becomes_a_newline_in_the_document() {
    let (mut session, _) = session("newline");
    type_keys(&mut session, "aa\\n");
    assert_eq!(session.document(), "ああ\n");
}

/// 半角英数モードでは、エンジンは打鍵をアプリへ素通しする。
/// CLI はそのアプリの役を務めるので、文字は文書に入る。
#[test]
fn ascii_mode_keystrokes_reach_the_document() {
    let (mut session, _) = session("ascii");
    type_keys(&mut session, "lhello world");
    assert_eq!(session.document(), "hello world");

    // Backspace も同じ経路でアプリに届く。
    type_keys(&mut session, "\\b");
    assert_eq!(session.document(), "hello worl");
}

#[test]
fn returning_from_ascii_mode_resumes_conversion() {
    let (mut session, _) = session("ascii-return");
    type_keys(&mut session, "labc");
    session.press(crystalskk_core::Key::Ctrl('j'));
    type_keys(&mut session, "Kanji\\s\\n");
    assert_eq!(session.document(), "abc漢字");
}

#[test]
fn saving_is_skipped_when_nothing_was_learned() {
    let (mut session, path) = session("clean");
    type_keys(&mut session, "kanji");
    assert!(!session.save_user_dictionary().expect("保存を試せる"));
    assert!(!path.exists(), "学習がなければ書き出さない");
}

#[test]
fn registered_words_are_offered_for_completion() {
    let (mut session, _) = session("complete-user");
    type_keys(&mut session, "Mikoto\\skotoba\\n");
    session.clear_document();

    // 登録した見出しが、ユーザー辞書から補完に出る。
    type_keys(&mut session, "Miko\\t");
    let view = session.completion().expect("補完している");
    assert!(
        view.entries.iter().any(|e| e.heading == "みこと"),
        "実際の候補: {:?}",
        view.entries
    );
}
