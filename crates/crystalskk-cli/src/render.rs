//! セッションの状態を文字列にする。
//!
//! 行入力モードと対話モードで同じ見え方にするため、整形はここに集める。

use crystalskk_core::engine::{CandidateView, Completed, CompletionView};
use crystalskk_settings::Settings;

use crate::session::Session;

/// 候補の見せ方。
///
/// **一覧を出す段階かどうかで見え方が変わる。** 一つずつ見せている間は
/// 選択中のものを囲み、一覧に移ったらラベル付きで並べる。実機の窓でも
/// 同じ切り替えをするので、CLI でも同じにしておく。
pub fn candidates(view: &CandidateView) -> String {
    if view.listing {
        return page(view);
    }

    let okuri = view.okuri.as_deref().unwrap_or("");
    let mut out = String::new();
    for (index, candidate) in view.candidates.iter().enumerate() {
        if index > 0 {
            out.push_str("  ");
        }
        let word = format!("{}{okuri}", candidate.word);
        if index == view.index {
            out.push_str(&format!("[{word}]"));
        } else {
            out.push_str(&format!(" {word} "));
        }
    }
    out
}

/// 一覧の一ページ。ラベルを添えて並べる。
///
/// 選ぶのはラベルキーを押すことなので、**どれが「選択中」かを示す囲みは
/// 付けない**。囲むと、矢印キーで動かせるかのように見えてしまう。
fn page(view: &CandidateView) -> String {
    let okuri = view.okuri.as_deref().unwrap_or("");
    let entries: Vec<String> = view
        .page()
        .into_iter()
        .map(|(label, candidate)| format!("{label}: {}{okuri}", candidate.word))
        .collect();
    format!(
        "{}   ({}/{} ページ)",
        entries.join("  "),
        view.page_number() + 1,
        view.page_count()
    )
}

/// 選んでいる補完候補。**出すのは変換先。** 読みは見れば大抵分かる。
///
/// 受け取る前は一行。Tab で巡り始めたら前後も並べる。端末では反転が使え
/// ないので、選んでいる候補を括弧でくくる。
pub fn completion(view: &CompletionView, settings: &Settings) -> String {
    let show = |entry: &Completed| {
        if settings.window.show_reading && entry.word != entry.heading {
            format!("{} ({})", entry.word, entry.heading)
        } else {
            entry.word.clone()
        }
    };
    if !view.taken {
        return format!(
            "{}: {}",
            settings.engine.completion.take_key,
            show(view.current())
        );
    }
    let page: Vec<String> = view
        .entries
        .iter()
        .enumerate()
        .map(|(at, entry)| {
            if at == view.current {
                format!("[{}]", show(entry))
            } else {
                show(entry)
            }
        })
        .collect();
    let mut text = page.join(" ");
    if view.count > 1 {
        text.push_str(&format!("  ({} / {})", view.number, view.count));
    }
    text
}

/// 選択中の候補の注釈。
///
/// 一覧を出している間は付けない。どれか一つを選んでいるわけではないので、
/// 一つぶんの注釈を出す先がない。
pub fn annotation(view: &CandidateView) -> Option<&str> {
    if view.listing {
        return None;
    }
    view.candidates.get(view.index)?.annotation.as_deref()
}

/// 文書の中身。改行は見えるようにする。
pub fn document(session: &Session) -> String {
    if session.document().is_empty() {
        "(なし)".to_owned()
    } else {
        session.document().replace('\n', "⏎")
    }
}

/// 未確定の表示。辞書登録中ならその見出しも添える。
pub fn preedit(session: &Session) -> String {
    let preedit = session.preedit();
    match session.registering() {
        Some(key) => {
            let depth = session.registration_depth();
            let nest = if depth > 1 {
                format!(" ×{depth}")
            } else {
                String::new()
            };
            if preedit.is_empty() {
                format!("[登録: {key}{nest}]")
            } else {
                format!("[登録: {key}{nest}] {preedit}")
            }
        }
        None if preedit.is_empty() => "(なし)".to_owned(),
        None => preedit,
    }
}

/// 入力モードの表示。
pub fn mode(session: &Session) -> String {
    let mode = session.mode();
    format!("{} {}", mode.label(), mode_name(mode))
}

fn mode_name(mode: crystalskk_core::InputMode) -> &'static str {
    use crystalskk_core::InputMode as M;
    match mode {
        M::Hiragana => "ひらがな",
        M::Katakana => "カタカナ",
        M::HalfKatakana => "半角カタカナ",
        M::FullAscii => "全角英数",
        M::Ascii => "半角英数",
    }
}
