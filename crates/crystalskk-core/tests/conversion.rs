//! 変換エンジンを打鍵列で駆動する試験。
//!
//! 一件ずつが「こう打ったらこうなる」という仕様の記述になるように書く。
//! 内部状態ではなく、確定した文字列・未確定の表示・発生した副作用だけを見る。

mod common;

use std::collections::HashMap;

use crystalskk_core::dict::{Candidate, CandidateSource, Query};
use crystalskk_core::engine::{Event, Marker};
use crystalskk_core::{Engine, InputMode, Key};

/// 試験用の固定辞書。
struct FixedDict(HashMap<String, Vec<Candidate>>);

impl FixedDict {
    fn new(entries: &[(&str, &[&str])]) -> Self {
        Self(
            entries
                .iter()
                .map(|(key, words)| {
                    (
                        (*key).to_owned(),
                        words.iter().map(|w| Candidate::new(*w)).collect(),
                    )
                })
                .collect(),
        )
    }
}

impl CandidateSource for FixedDict {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.0.get(&query.key).cloned().unwrap_or_default()
    }
}

/// 打鍵とその結果を溜めておく試験用の入力セッション。
struct Session {
    engine: Engine,
    committed: String,
    events: Vec<Event>,
}

impl Session {
    fn new() -> Self {
        let dict = FixedDict::new(&[
            ("かんじ", &["漢字", "感じ", "幹事"][..]),
            ("おくr", &["送"][..]),
            ("たべr", &["食べ"][..]),
            ("かよu", &["通"][..]),
            ("ほん", &["本"][..]),
            ("skk", &["SKK"][..]),
            ("ことば", &["言葉"][..]),
            ("ぱそこん", &["パソコン", "パソ魂"][..]),
        ]);
        Self {
            engine: common::engine(Box::new(dict)),
            committed: String::new(),
            events: Vec::new(),
        }
    }

    /// 一打鍵。
    fn press(&mut self, key: Key) -> bool {
        let response = self.engine.press(key);
        self.committed.push_str(&response.commit);
        self.events.extend(response.events);
        response.handled
    }

    /// 打鍵列。英大文字はシフト付き、空白は Space、`\n` は Enter、
    /// `\x08` は Backspace として送る。
    fn type_keys(&mut self, keys: &str) -> &mut Self {
        for c in keys.chars() {
            let key = match c {
                ' ' => Key::Space,
                '\n' => Key::Enter,
                '\u{8}' => Key::Backspace,
                c => Key::Char(c),
            };
            self.press(key);
        }
        self
    }

    /// 印を含めた未確定表示。
    fn preedit(&self) -> String {
        self.engine.preedit().display()
    }

    fn marker(&self) -> Marker {
        self.engine.preedit().marker
    }
}

#[test]
fn hiragana_is_committed_as_you_type() {
    let mut s = Session::new();
    s.type_keys("kanji");
    assert_eq!(s.committed, "かんじ");
    assert_eq!(s.preedit(), "");
}

#[test]
fn shift_starts_a_midashi() {
    let mut s = Session::new();
    s.type_keys("Kanji");
    // 見出し語の入力中はまだ何も確定していない。
    assert_eq!(s.committed, "");
    assert_eq!(s.preedit(), "▽かんじ");
    assert_eq!(s.marker(), Marker::Composing);
}

#[test]
fn space_converts_and_enter_commits() {
    let mut s = Session::new();
    s.type_keys("Kanji ");
    assert_eq!(s.preedit(), "▼漢字");
    assert_eq!(s.marker(), Marker::Selecting);

    s.type_keys("\n");
    assert_eq!(s.committed, "漢字");
    assert_eq!(s.preedit(), "");
    assert_eq!(
        s.events,
        [Event::Learn {
            query: Query::okuri_nashi("かんじ"),
            word: "漢字".into()
        }]
    );
}

#[test]
fn space_walks_through_candidates() {
    let mut s = Session::new();
    s.type_keys("Kanji ");
    assert_eq!(s.preedit(), "▼漢字");
    s.type_keys(" ");
    assert_eq!(s.preedit(), "▼感じ");
    s.type_keys(" ");
    assert_eq!(s.preedit(), "▼幹事");
    // x で前の候補へ戻る。
    s.type_keys("x");
    assert_eq!(s.preedit(), "▼感じ");
}

#[test]
fn x_at_the_first_candidate_returns_to_the_midashi() {
    let mut s = Session::new();
    s.type_keys("Kanji x");
    assert_eq!(s.preedit(), "▽かんじ");
    assert_eq!(s.committed, "");
}

#[test]
fn candidate_window_reflects_the_selection() {
    let mut s = Session::new();
    s.type_keys("Kanji ");
    let view = s
        .engine
        .press(Key::Space)
        .candidates
        .expect("候補選択中なら候補が出る");
    assert_eq!(view.index, 1);
    let words: Vec<&str> = view.candidates.iter().map(|c| c.word.as_str()).collect();
    assert_eq!(words, ["漢字", "感じ", "幹事"]);
}

#[test]
fn okuri_converts_without_pressing_space() {
    let mut s = Session::new();
    s.type_keys("OkuRi");
    // 送り仮名が一文字確定した時点で変換が起きる。
    assert_eq!(s.preedit(), "▼送り");
    s.type_keys("\n");
    assert_eq!(s.committed, "送り");
    assert_eq!(
        s.events,
        [Event::Learn {
            query: Query::okuri_ari("おく", 'r', "り"),
            word: "送".into()
        }]
    );
}

#[test]
fn okuri_shows_the_separator_while_incomplete() {
    let mut s = Session::new();
    s.type_keys("TabeR");
    assert_eq!(s.preedit(), "▽たべ*r");
    s.type_keys("u");
    assert_eq!(s.preedit(), "▼食べる");
}

/// シフトの押し遅れを救う。
///
/// `KayoU` と打つべきところを `kAyoU` と打ってしまう取り違えは起きやすい。
/// 単独ではかなにならない `k` を捨てずに残し、続く `a` と組み合わせる。
#[test]
fn a_late_shift_does_not_lose_the_consonant() {
    let mut correct = Session::new();
    correct.type_keys("KayoU");
    assert_eq!(correct.preedit(), "▼通う");

    let mut mistyped = Session::new();
    mistyped.type_keys("kAyoU");
    assert_eq!(mistyped.preedit(), "▼通う", "打ち間違いでも同じ結果になる");
    assert_eq!(mistyped.committed, "", "取りこぼした打鍵が確定されない");
}

#[test]
fn a_late_shift_is_rescued_mid_word_too() {
    let mut s = Session::new();
    // `Kanji` のつもりで `kAnji` と打つ。
    s.type_keys("kAnji");
    assert_eq!(s.preedit(), "▽かんじ");
    assert_eq!(s.committed, "");
}

/// シフトの押しすぎを救う。
///
/// 勢いあまって一つの音の全部を大文字にしてしまうことがある。二度目以降の
/// シフトは新しい区切りを作れない位置にあるので、意味を持たない。
#[test]
fn an_extra_shift_inside_a_syllable_is_ignored() {
    let mut s = Session::new();
    s.type_keys("KAyoU");
    assert_eq!(s.preedit(), "▼通う", "KayoU と同じ結果になる");
    assert_eq!(s.committed, "");
}

#[test]
fn an_extra_shift_does_not_start_okuri_before_the_midashi() {
    let mut s = Session::new();
    // 見出し語にまだかながないので、二文字目のシフトは送り仮名を始められない。
    s.type_keys("KAnji");
    assert_eq!(s.preedit(), "▽かんじ");
}

#[test]
fn an_extra_shift_inside_okuri_is_ignored() {
    let mut s = Session::new();
    // 送り仮名はすでに始まっているので、`U` は送り仮名の続きでしかない。
    s.type_keys("TabeRU");
    assert_eq!(s.preedit(), "▼食べる");
}

/// 単独でかなになる打鍵は、シフトの前に確定させる。
///
/// `honYa` (本屋) では `n` は `ん` として確定すべきであり、続く `ya` と
/// 組み合わせて `にゃ` にしてはいけない。
#[test]
fn a_standalone_kana_before_the_shift_is_committed() {
    let mut s = Session::new();
    s.type_keys("honYa");
    assert_eq!(s.committed, "ほん");
    assert_eq!(s.preedit(), "▽や");
}

/// 送り仮名の前でも同じ救済が効く。
///
/// 送り仮名は子音から始まるので、その子音を打ってからシフトすると
/// 辞書キーの末尾もその子音になる。
#[test]
fn a_late_shift_before_okuri_keeps_the_consonant() {
    let mut correct = Session::new();
    correct.type_keys("OkuRi");
    assert_eq!(correct.preedit(), "▼送り");

    let mut mistyped = Session::new();
    mistyped.type_keys("OkurI");
    assert_eq!(
        mistyped.preedit(),
        "▼送り",
        "送り仮名の子音を打ってからシフトしても同じ"
    );
}

#[test]
fn a_standalone_kana_before_okuri_joins_the_midashi() {
    let mut s = Session::new();
    // `ん` は単独で成立するので見出し語に入り、送り仮名は `じ` から始まる。
    s.type_keys("HonJ");
    assert_eq!(s.preedit(), "▽ほん*j");

    // 送り仮名が確定すると、辞書キーは見出し語 + 送り仮名の子音になる。
    s.type_keys("i");
    assert_eq!(s.engine.preedit().registering.as_deref(), Some("ほんj"));
}

#[test]
fn typing_on_a_candidate_commits_it_implicitly() {
    let mut s = Session::new();
    s.type_keys("Kanji ");
    // 確定操作をせずに次の入力を始めると、候補はそのまま確定する。
    s.type_keys("no");
    assert_eq!(s.committed, "漢字の");
    assert_eq!(s.preedit(), "");
}

#[test]
fn abbrev_looks_up_ascii_directly() {
    let mut s = Session::new();
    s.type_keys("/skk ");
    assert_eq!(s.preedit(), "▼SKK");
    s.type_keys("\n");
    assert_eq!(s.committed, "SKK");
}

#[test]
fn q_commits_the_midashi_as_katakana() {
    let mut s = Session::new();
    s.type_keys("Kanjiq");
    assert_eq!(s.committed, "カンジ");
    assert_eq!(s.preedit(), "");
}

#[test]
fn a_katakana_that_is_already_a_candidate_is_learned() {
    // **並べ替えであって、新しい語ではない。** 次は space でも同じものが
    // 先に出てほしい。
    let mut s = Session::new();
    s.type_keys("Pasokonq");
    assert_eq!(s.committed, "パソコン");
    assert_eq!(
        s.events,
        [Event::Learn {
            query: Query::okuri_nashi("ぱそこん"),
            word: "パソコン".into()
        }]
    );
}

#[test]
fn a_katakana_that_is_not_a_candidate_is_not_learned() {
    // 「かんじ」は辞書にあるが「カンジ」は候補に無い。覚えれば**辞書に
    // 無い語を作ってしまう。**
    let mut s = Session::new();
    s.type_keys("Kanjiq");
    assert_eq!(s.committed, "カンジ");
    assert!(s.events.is_empty());
}

#[test]
fn an_unknown_heading_learns_nothing() {
    let mut s = Session::new();
    s.type_keys("Nanikaq");
    assert_eq!(s.committed, "ナニカ");
    assert!(s.events.is_empty());
}

#[test]
fn ctrl_q_commits_the_midashi_as_halfwidth_katakana() {
    let mut s = Session::new();
    s.type_keys("Kanji");
    s.press(Key::Ctrl('q'));
    assert_eq!(s.committed, "ｶﾝｼﾞ");
    assert_eq!(s.preedit(), "");
    // モードは変わらない。確定の字種を選んだだけ。
    assert_eq!(s.engine.mode(), InputMode::Hiragana);
}

#[test]
fn ctrl_q_takes_the_okuri_along() {
    let mut s = Session::new();
    s.type_keys("OkuR");
    s.press(Key::Ctrl('q'));
    assert_eq!(s.committed, "ｵｸ");
}

#[test]
fn ctrl_q_toggles_halfwidth_katakana_mode_in_direct_input() {
    let mut s = Session::new();
    s.press(Key::Ctrl('q'));
    assert_eq!(s.engine.mode(), InputMode::HalfKatakana);
    s.type_keys("kanji");
    assert_eq!(s.committed, "ｶﾝｼﾞ");

    s.press(Key::Ctrl('q'));
    assert_eq!(s.engine.mode(), InputMode::Hiragana);
}

#[test]
fn q_returns_to_hiragana_from_halfwidth_katakana() {
    let mut s = Session::new();
    s.press(Key::Ctrl('q'));
    assert_eq!(s.engine.mode(), InputMode::HalfKatakana);
    // `q` はどのかなモードからでもひらがなへ帰る手段になる。
    s.type_keys("q");
    assert_eq!(s.engine.mode(), InputMode::Hiragana);
}

#[test]
fn ctrl_q_on_a_candidate_commits_it_first() {
    let mut s = Session::new();
    s.type_keys("Kanji ");
    assert_eq!(s.preedit(), "▼漢字");
    s.press(Key::Ctrl('q'));
    assert_eq!(s.committed, "漢字", "選んでいた候補はそのまま確定する");
    assert_eq!(s.engine.mode(), InputMode::HalfKatakana);
}

#[test]
fn q_toggles_katakana_mode_in_direct_input() {
    let mut s = Session::new();
    s.type_keys("q");
    assert_eq!(s.engine.mode(), InputMode::Katakana);
    s.type_keys("kanji");
    assert_eq!(s.committed, "カンジ");
    s.type_keys("q");
    assert_eq!(s.engine.mode(), InputMode::Hiragana);
}

#[test]
fn ascii_mode_passes_keys_through() {
    let mut s = Session::new();
    s.type_keys("l");
    assert_eq!(s.engine.mode(), InputMode::Ascii);
    assert!(!s.press(Key::Char('a')), "英数モードの打鍵はアプリに渡す");
    assert_eq!(s.committed, "");

    s.press(Key::Ctrl('j'));
    assert_eq!(s.engine.mode(), InputMode::Hiragana);
}

#[test]
fn fullwidth_ascii_mode_commits_wide_characters() {
    let mut s = Session::new();
    s.type_keys("L");
    assert_eq!(s.engine.mode(), InputMode::FullAscii);
    s.type_keys("Ab1");
    assert_eq!(s.committed, "Ａｂ１");
}

#[test]
fn ctrl_g_abandons_the_midashi() {
    let mut s = Session::new();
    s.type_keys("Kanji");
    s.press(Key::Ctrl('g'));
    assert_eq!(s.preedit(), "");
    assert_eq!(s.committed, "");
}

#[test]
fn backspace_walks_back_through_the_midashi() {
    let mut s = Session::new();
    s.type_keys("Kanj");
    assert_eq!(s.preedit(), "▽かんj");
    // 未確定のローマ字がまず削られる。
    s.type_keys("\u{8}");
    assert_eq!(s.preedit(), "▽かん");
    s.type_keys("\u{8}");
    assert_eq!(s.preedit(), "▽か");
    // 読みを消し切っても `▽` は残る。印だけ残して打ち直せる。
    s.type_keys("\u{8}");
    assert_eq!(s.preedit(), "▽");
    s.type_keys("ki");
    assert_eq!(s.preedit(), "▽き");
    // 空の `▽` でもう一度押すと直接入力へ戻る。
    s.type_keys("\u{8}\u{8}");
    assert_eq!(s.preedit(), "");
    assert_eq!(s.marker(), Marker::None);
}

#[test]
fn converting_an_empty_midashi_leaves_the_midashi() {
    // 引くものが無いので、登録を始めずに `▽` を抜ける。ddskk と同じ。
    let mut s = Session::new();
    s.type_keys("K\u{8} ");
    assert_eq!(s.preedit(), "");
    assert_eq!(s.marker(), Marker::None);
    assert_eq!(s.committed, "");
}

#[test]
fn backspace_in_the_okuri_takes_the_separator_too() {
    // 区切りだけ残っても、続けて打つかもう一度消すしかない。
    let mut s = Session::new();
    s.type_keys("OkuR");
    assert_eq!(s.preedit(), "▽おく*r");
    s.type_keys("\u{8}");
    assert_eq!(s.preedit(), "▽おく");
    s.type_keys("Ri");
    assert_eq!(
        s.preedit(),
        "▼送り",
        "区切りを消した後も送り仮名を打ち直せる"
    );
}

#[test]
fn backspace_while_selecting_commits_all_but_the_last_character() {
    // ddskk の `skk-delete-implies-kakutei` の既定と同じ。確定した後は
    // ただの文字なので、送り仮名の区切りも残らない。
    let mut s = Session::new();
    s.type_keys("OkuRi\u{8}");
    assert_eq!(s.committed, "送");
    assert_eq!(s.preedit(), "");
    // 選んだ候補は正しいので、学習はする。
    assert_eq!(
        s.events,
        [Event::Learn {
            query: Query::okuri_ari("おく", 'r', "り"),
            word: "送".into()
        }]
    );
}

#[test]
fn x_still_steps_back_to_the_midashi() {
    // Backspace が確定に変わっても、前の候補へ戻る道は x に残る。
    let mut s = Session::new();
    s.type_keys("OkuRix");
    assert_eq!(s.preedit(), "▽おくり");
    assert_eq!(s.committed, "");
}

#[test]
fn unknown_word_enters_registration() {
    let mut s = Session::new();
    s.type_keys("Mikoto ");
    assert_eq!(s.engine.registration_depth(), 1);
    assert_eq!(s.engine.preedit().registering.as_deref(), Some("みこと"));
    assert_eq!(s.committed, "", "登録中はアプリへ何も送らない");

    // 登録中の入力は直接入力と同じに振る舞い、結果だけが溜まる。
    s.type_keys("kotoba");
    assert_eq!(s.committed, "");

    s.type_keys("\n");
    assert_eq!(s.committed, "ことば");
    assert_eq!(s.engine.registration_depth(), 0);
    assert_eq!(
        s.events,
        [Event::Register {
            query: Query::okuri_nashi("みこと"),
            word: "ことば".into()
        }]
    );
}

#[test]
fn registration_can_convert_and_nest() {
    let mut s = Session::new();
    s.type_keys("Mikoto ");
    assert_eq!(s.engine.registration_depth(), 1);

    // 登録中にさらに未知語を変換しようとすると、登録が入れ子になる。
    s.type_keys("Shiranai ");
    assert_eq!(s.engine.registration_depth(), 2);
    assert_eq!(s.engine.preedit().registering.as_deref(), Some("しらない"));

    s.type_keys("kotoba\n");
    assert_eq!(s.engine.registration_depth(), 1);
    assert_eq!(s.committed, "", "内側の登録結果は外側の枠に溜まる");

    s.type_keys("\n");
    assert_eq!(s.committed, "ことば");
    assert_eq!(s.engine.registration_depth(), 0);
}

#[test]
fn registration_uses_converted_words_too() {
    let mut s = Session::new();
    s.type_keys("Mikoto ");
    // 登録語そのものを変換して作れる。
    s.type_keys("Kanji \n");
    assert_eq!(s.committed, "");
    s.type_keys("\n");
    assert_eq!(s.committed, "漢字");
    assert!(s.events.contains(&Event::Register {
        query: Query::okuri_nashi("みこと"),
        word: "漢字".into(),
    }));
}

#[test]
fn empty_registration_returns_to_the_midashi() {
    let mut s = Session::new();
    s.type_keys("Mikoto ");
    // 何も入力せずに確定すると登録は成立せず、見出し語入力へ戻る。
    s.type_keys("\n");
    assert_eq!(s.engine.registration_depth(), 0);
    assert_eq!(s.preedit(), "▽みこと");
    assert_eq!(s.committed, "");
    assert!(s.events.is_empty());
}

#[test]
fn exhausting_candidates_enters_registration() {
    let mut s = Session::new();
    s.type_keys("Kanji ");
    // 候補は3件。出し切った次の Space で登録に入る。
    s.type_keys("   ");
    assert_eq!(s.engine.registration_depth(), 1);
    assert_eq!(s.engine.preedit().registering.as_deref(), Some("かんじ"));
}

#[test]
fn okuri_registration_keeps_the_okuri_out_of_the_registered_word() {
    let mut s = Session::new();
    s.type_keys("HashiRu");
    assert_eq!(s.engine.registration_depth(), 1);
    assert_eq!(s.engine.preedit().registering.as_deref(), Some("はしr"));

    s.type_keys("kakeru\n");
    // 確定文字列には送り仮名が付くが、辞書に登録されるのは語幹だけ。
    assert_eq!(s.committed, "かけるる");
    assert_eq!(
        s.events,
        [Event::Register {
            query: Query::okuri_ari("はし", 'r', "る"),
            word: "かける".into()
        }]
    );
}

#[test]
fn reset_discards_everything_but_the_mode() {
    let mut s = Session::new();
    s.type_keys("q");
    s.type_keys("Mikoto ");
    assert_eq!(s.engine.registration_depth(), 1);

    s.engine.reset();
    assert_eq!(s.preedit(), "");
    assert_eq!(s.engine.registration_depth(), 0);
    assert!(s.engine.preedit().registering.is_none());
    assert_eq!(
        s.engine.mode(),
        InputMode::Katakana,
        "モードは利用者の設定なので残す"
    );
}

#[test]
fn enter_without_pending_input_is_left_to_the_application() {
    let mut s = Session::new();
    assert!(!s.press(Key::Enter), "未確定がなければ改行はアプリの仕事");

    // 未確定の `ん` があるときは、それを確定させて改行は送らない。
    s.type_keys("hon");
    assert!(s.press(Key::Enter));
    assert_eq!(s.committed, "ほん");
}
