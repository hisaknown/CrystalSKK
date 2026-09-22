//! 打鍵列の表記。
//!
//! 行入力モードでは、一行がそのまま打鍵列になる。印字できないキーは
//! 逆斜線とキャレットで表す。
//!
//! | 表記 | キー |
//! |---|---|
//! | 印字可能文字 | その文字。英大文字はシフト付きの打鍵 |
//! | 空白 | Space |
//! | `\n` | Enter |
//! | `\b` | Backspace |
//! | `\t` | Tab |
//! | `\e` | Escape |
//! | `\u` / `\d` | ↑ / ↓ |
//! | `^X` | Ctrl+X |
//! | `\\` / `\^` | `\` / `^` そのもの |

use crystalskk_core::Key;

/// 表記を打鍵列に直す。
pub fn parse(source: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut chars = source.chars().peekable();

    while let Some(c) = chars.next() {
        let key = match c {
            '\\' => match chars.next() {
                Some('n') => Key::Enter,
                Some('b') => Key::Backspace,
                Some('t') => Key::Tab,
                Some('e') => Key::Escape,
                Some('u') => Key::Up,
                Some('d') => Key::Down,
                Some('s') => Key::Space,
                Some('\\') => Key::Char('\\'),
                Some('^') => Key::Char('^'),
                Some(other) => return Err(format!("知らない表記です: \\{other}")),
                None => return Err("行末に \\ だけが残っています".to_owned()),
            },
            '^' => match chars.next() {
                Some(c) if c.is_ascii_alphabetic() => Key::Ctrl(c.to_ascii_lowercase()),
                Some(other) => return Err(format!("Ctrl に続けられません: ^{other}")),
                None => return Err("行末に ^ だけが残っています".to_owned()),
            },
            ' ' => Key::Space,
            c => Key::Char(c),
        };
        keys.push(key);
    }
    Ok(keys)
}

/// 打鍵一つを表記に戻す。入力の読み上げや記録に使う。
pub fn display(key: Key) -> String {
    match key {
        Key::Char('\\') => "\\\\".to_owned(),
        Key::Char('^') => "\\^".to_owned(),
        Key::Char(c) => c.to_string(),
        Key::Space => "␣".to_owned(),
        Key::Enter => "⏎".to_owned(),
        Key::Backspace => "⌫".to_owned(),
        Key::Escape => "esc".to_owned(),
        Key::Tab => "⇥".to_owned(),
        Key::Up => "↑".to_owned(),
        Key::Down => "↓".to_owned(),
        Key::Ctrl(c) => format!("^{}", c.to_ascii_uppercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_characters_become_char_keys() {
        assert_eq!(
            parse("Ka").expect("読める"),
            [Key::Char('K'), Key::Char('a')]
        );
    }

    #[test]
    fn a_space_is_the_space_key() {
        assert_eq!(
            parse("a b").expect("読める"),
            [Key::Char('a'), Key::Space, Key::Char('b')]
        );
    }

    #[test]
    fn escapes_name_the_unprintable_keys() {
        assert_eq!(
            parse("\\n\\b\\t\\e\\u\\d").expect("読める"),
            [
                Key::Enter,
                Key::Backspace,
                Key::Tab,
                Key::Escape,
                Key::Up,
                Key::Down
            ]
        );
    }

    #[test]
    fn caret_marks_control_keys() {
        assert_eq!(
            parse("^J^g").expect("読める"),
            [Key::Ctrl('j'), Key::Ctrl('g')]
        );
    }

    #[test]
    fn literal_backslash_and_caret_need_escaping() {
        assert_eq!(
            parse("\\\\\\^").expect("読める"),
            [Key::Char('\\'), Key::Char('^')]
        );
    }

    #[test]
    fn reports_what_it_cannot_read() {
        assert!(parse("\\z").is_err());
        assert!(parse("\\").is_err());
        assert!(parse("^").is_err());
        assert!(parse("^1").is_err());
    }

    #[test]
    fn display_round_trips_through_parse() {
        for key in [Key::Char('a'), Key::Char('\\'), Key::Char('^')] {
            assert_eq!(parse(&display(key)).expect("読める"), [key]);
        }
    }
}
