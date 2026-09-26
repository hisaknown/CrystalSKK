//! `[keys]` の節。キーの書き方を読む。
//!
//! キーは `"Ctrl+J"` のように書く。Windows の画面やメニューで見かける
//! 書き方である。一つの操作に幾つでも割り当てられるので、値は配列にする。
//!
//! 割り当ては二種類ある (ADR-0038)。
//!
//! - **変換の操作** (確定、取り消し、…)。エンジンが解釈する。書けるのは
//!   エンジンが受け取れるキーだけで、文字、`Ctrl+` と英字、名前の付いた
//!   キー (Space、Tab など) である。
//! - **入切の操作**。TSF に横取りを頼むキーで、切られていても届く。
//!   こちらは `半角/全角` や `F1` のような、文字にならないキーも書ける。

use crystalskk_core::{Command, Key, Keymap, RomajiTable};
use toml_edit::Table;

use crate::Error;

/// 入切のキー。**TIP だけが使う。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnOffKeys {
    /// 入と切を兼ねるキー。
    pub toggle: Vec<Hotkey>,
    /// 入にするだけのキー。
    pub on: Vec<Hotkey>,
    /// 切にするだけのキー。
    pub off: Vec<Hotkey>,
}

/// 入切のキー一つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: HotkeyKey,
}

/// 入切のキーの、修飾を除いた部分。
///
/// 仮想キーには直さない。文字のキーがどの仮想キーかはキーボード配列で
/// 決まるので、直すのは配列を知っている TIP の仕事である。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyKey {
    /// 半角/全角。配列や修飾によって幾つかの仮想キーで届くので、修飾を
    /// 問わずに全部拾う。
    HankakuZenkaku,
    /// IME を入にする専用のキー。持っている配列だけが送ってくる。
    ImeOn,
    /// IME を切にする専用のキー。
    ImeOff,
    Space,
    /// ファンクションキー (1 から 24)。
    Function(u8),
    /// 文字を打つキー。
    Char(char),
}

/// 変換の操作に割り当てたキーを読む。
pub fn keymap(table: &Table) -> Result<Keymap, Error> {
    let mut bindings: Vec<(Key, Command, &str)> = Vec::new();
    for (command, name) in Command::ALL {
        for written in list(table, name)? {
            let key = engine_key(&written).ok_or_else(|| {
                Error::new(format!(
                    "keys.{name} の「{written}」は使えないキーです \
                     (例: \"q\"、\"Ctrl+J\"、\"Space\"、\"Tab\")"
                ))
            })?;
            if let Some((_, _, other)) = bindings.iter().find(|(bound, _, _)| *bound == key) {
                // 一つのキーで二つのことはできない。どちらが勝つかを黙って
                // 決めると、利用者には片方が効かない理由が分からない。
                return Err(Error::new(format!(
                    "「{written}」が keys.{other} と keys.{name} の両方にあります"
                )));
            }
            bindings.push((key, command, name));
        }
    }
    Ok(Keymap::new(
        bindings
            .into_iter()
            .map(|(key, command, _)| (key, command))
            .collect(),
    )
    .expect("重なりはいま確かめた"))
}

/// 入切のキーを読む。
pub fn on_off(table: &Table) -> Result<OnOffKeys, Error> {
    let read = |name: &str| -> Result<Vec<Hotkey>, Error> {
        list(table, name)?
            .iter()
            .map(|written| {
                hotkey(written).ok_or_else(|| {
                    Error::new(format!(
                        "keys.{name} の「{written}」は使えないキーです \
                         (例: \"半角/全角\"、\"Alt+`\"、\"Ctrl+Space\"、\"F12\")"
                    ))
                })
            })
            .collect()
    };
    let keys = OnOffKeys {
        toggle: read("on_off")?,
        on: read("on")?,
        off: read("off")?,
    };
    if keys.toggle.is_empty() && keys.on.is_empty() {
        // 入にする手立てが無ければ、切られたアプリでは二度と入にできない。
        return Err(Error::new(
            "keys.on_off か keys.on に、入にするキーを一つは書いてください",
        ));
    }
    Ok(keys)
}

/// キーの名前の配列。空でもよい。
fn list(table: &Table, name: &str) -> Result<Vec<String>, Error> {
    let wrong = || {
        Error::new(format!(
            "keys.{name} はキーの名前の配列で書いてください (例: [\"Ctrl+J\"])"
        ))
    };
    let array = crate::value(table, "keys", name)?
        .as_array()
        .ok_or_else(wrong)?;
    array
        .iter()
        .map(|item| item.as_str().map(str::to_owned).ok_or_else(wrong))
        .collect()
}

/// 修飾と、残りの部分に分ける。`"Ctrl+J"` は (Ctrl, `"J"`)。
///
/// `+` そのものも書けるよう、最後の `+` の後ろが空なら `+` をキーとみなす
/// (`"Ctrl++"`、`"+"`)。
fn split(written: &str) -> Option<(Modifiers, &str)> {
    let mut modifiers = Modifiers::default();
    let mut rest = written;
    while let Some(at) = rest.find('+') {
        if at + 1 == rest.len() {
            break;
        }
        let (head, tail) = (&rest[..at], &rest[at + 1..]);
        match head.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.ctrl = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            _ => return None,
        }
        rest = tail;
    }
    (!rest.is_empty()).then_some((modifiers, rest))
}

#[derive(Debug, Default, Clone, Copy)]
struct Modifiers {
    ctrl: bool,
    shift: bool,
    alt: bool,
}

/// 一文字なら、その文字。
fn single(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// エンジンのキーとして読む。
///
/// シフトは書かない。英字の大文字がシフト付きの打鍵である (`"X"`)。
fn engine_key(written: &str) -> Option<Key> {
    let (modifiers, rest) = split(written)?;
    if modifiers.shift || modifiers.alt {
        return None;
    }
    if modifiers.ctrl {
        let c = single(rest)?;
        return c
            .is_ascii_alphabetic()
            .then(|| Key::Ctrl(c.to_ascii_lowercase()));
    }
    let named = match rest.to_ascii_lowercase().as_str() {
        "space" => Some(Key::Space),
        "enter" => Some(Key::Enter),
        "backspace" => Some(Key::Backspace),
        "escape" | "esc" => Some(Key::Escape),
        "tab" => Some(Key::Tab),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    single(rest)
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .map(Key::Char)
}

/// 入切のキーとして読む。
fn hotkey(written: &str) -> Option<Hotkey> {
    let (modifiers, rest) = split(written)?;
    let lowered = rest.to_ascii_lowercase();
    let key = match lowered.as_str() {
        "半角/全角" | "hankaku/zenkaku" => HotkeyKey::HankakuZenkaku,
        "imeon" => HotkeyKey::ImeOn,
        "imeoff" => HotkeyKey::ImeOff,
        "space" => HotkeyKey::Space,
        _ => match lowered.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
            Some(n) if (1..=24).contains(&n) => HotkeyKey::Function(n),
            _ => HotkeyKey::Char(single(rest).filter(|c| !c.is_whitespace() && !c.is_control())?),
        },
    };
    // 半角/全角と IME 専用のキーは修飾を問わずに拾う。書かれても意味が無い。
    let bare = matches!(
        key,
        HotkeyKey::HankakuZenkaku | HotkeyKey::ImeOn | HotkeyKey::ImeOff
    );
    if bare && (modifiers.ctrl || modifiers.shift || modifiers.alt) {
        return None;
    }
    // 修飾の無い文字や空白を横取りすると、その文字が打てなくなる。
    if !bare
        && !modifiers.ctrl
        && !modifiers.alt
        && matches!(key, HotkeyKey::Char(_) | HotkeyKey::Space)
    {
        return None;
    }
    Some(Hotkey {
        ctrl: modifiers.ctrl,
        shift: modifiers.shift,
        alt: modifiers.alt,
        key,
    })
}

/// 文字のキーの割り当てのうち、ローマ字の規則とぶつかるものを挙げる。
///
/// **断らずに知らせるだけにする。** 使わない規則の頭文字を操作に回すのは、
/// 意図してのことでもありうる。知らせるのは、利用者が自分で設定を読み
/// 直したときだけである (ADR-0038)。
///
/// ぶつかるのは、直接入力か見出し語入力で働く操作に割り当てた文字だけで
/// ある。小文字ならその文字で始まる規則が打てなくなり、大文字ならその文字で
/// 見出し語や送り仮名を始められなくなる。打ちかけの途中ではローマ字が勝つ
/// ので、二文字目以降に出てくる規則は困らない。
pub fn clashes(keymap: &Keymap, romaji: &RomajiTable) -> Vec<String> {
    let mut found = Vec::new();
    for (command, name) in Command::ALL {
        if !command.acts_while_typing() {
            continue;
        }
        for key in keymap.keys(command) {
            let Key::Char(c) = key else {
                continue;
            };
            let head = c.to_ascii_lowercase();
            let Some(rule) = romaji.rules().find(|rule| rule.input.starts_with(head)) else {
                continue;
            };
            found.push(if c.is_ascii_uppercase() {
                format!(
                    "keys.{name} の \"{c}\" を割り当てているので、{c} で見出し語や送り仮名を始められません ({head} で始まる規則 {} があります)",
                    rule.input
                )
            } else {
                format!(
                    "keys.{name} の \"{c}\" を割り当てているので、{c} で始まる規則 ({}) を打てません",
                    rule.input
                )
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_keys_are_read_as_written() {
        assert_eq!(engine_key("Ctrl+J"), Some(Key::Ctrl('j')));
        assert_eq!(engine_key("ctrl+j"), Some(Key::Ctrl('j')));
        assert_eq!(engine_key("q"), Some(Key::Char('q')));
        assert_eq!(engine_key("X"), Some(Key::Char('X')));
        assert_eq!(engine_key("+"), Some(Key::Char('+')));
        assert_eq!(engine_key("Space"), Some(Key::Space));
        assert_eq!(engine_key("Tab"), Some(Key::Tab));
        assert_eq!(engine_key("Esc"), Some(Key::Escape));
    }

    #[test]
    fn engine_keys_it_cannot_receive_are_refused() {
        for written in ["", "Ctrl+1", "Alt+J", "Shift+x", "Hyper+J", "ab", " ", "F1"] {
            assert_eq!(engine_key(written), None, "{written:?}");
        }
    }

    #[test]
    fn on_off_keys_are_read_as_written() {
        let hotkey = |written| hotkey(written).map(|h| (h.ctrl, h.shift, h.alt, h.key));
        assert_eq!(
            hotkey("半角/全角"),
            Some((false, false, false, HotkeyKey::HankakuZenkaku))
        );
        assert_eq!(
            hotkey("Alt+`"),
            Some((false, false, true, HotkeyKey::Char('`')))
        );
        assert_eq!(
            hotkey("Ctrl+Space"),
            Some((true, false, false, HotkeyKey::Space))
        );
        assert_eq!(
            hotkey("F12"),
            Some((false, false, false, HotkeyKey::Function(12)))
        );
        assert_eq!(
            hotkey("ImeOn"),
            Some((false, false, false, HotkeyKey::ImeOn))
        );
    }

    #[test]
    fn on_off_keys_that_would_eat_typing_are_refused() {
        for written in ["a", "Shift+a", "Space", "Ctrl+半角/全角", "F25", "Win+a"] {
            assert_eq!(hotkey(written), None, "{written:?}");
        }
    }

    fn clashes_in(keys: &str, romaji: &str) -> Vec<String> {
        let template = crate::TEMPLATE.replace("toggle_kana = [\"q\"]", keys);
        let settings = crate::parse(&template, romaji).unwrap();
        clashes(&settings.engine.keys, &settings.engine.romaji)
    }

    #[test]
    fn the_template_clashes_with_nothing() {
        assert!(clashes_in("toggle_kana = [\"q\"]", crate::ROMAJI_TEMPLATE).is_empty());
    }

    #[test]
    fn a_letter_that_starts_a_rule_is_told() {
        let romaji = format!("{}qa\tくぁ\n", crate::ROMAJI_TEMPLATE);
        let found = clashes_in("toggle_kana = [\"q\"]", &romaji);
        // q だけでなく、set_henkan_point の Q もぶつかるようになる。
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].contains("(qa) を打てません"), "{}", found[0]);
        assert!(found[1].contains("\"Q\""), "{}", found[1]);
    }

    #[test]
    fn a_capital_that_starts_a_rule_is_told() {
        let found = clashes_in("toggle_kana = [\"K\"]", crate::ROMAJI_TEMPLATE);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("見出し語"), "{}", found[0]);
    }

    #[test]
    fn keys_used_only_while_selecting_are_left_alone() {
        let template = crate::TEMPLATE.replace("purge = [\"X\"]", "purge = [\"K\"]");
        let settings = crate::parse(&template, crate::ROMAJI_TEMPLATE).unwrap();
        assert!(clashes(&settings.engine.keys, &settings.engine.romaji).is_empty());
    }
}
