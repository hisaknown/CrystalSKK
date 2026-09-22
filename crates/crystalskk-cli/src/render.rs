//! セッションの状態を文字列にする。
//!
//! 行入力モードと対話モードで同じ見え方にするため、整形はここに集める。

use crystalskk_core::engine::CandidateView;

use crate::session::Session;

/// 候補一覧。選択中のものを囲んで示す。
pub fn candidates(view: &CandidateView) -> String {
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

/// 選択中の候補の注釈。
pub fn annotation(view: &CandidateView) -> Option<&str> {
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
