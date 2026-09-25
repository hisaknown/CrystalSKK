//! エンジンへ与えるキー入力。
//!
//! OS のキーコードには依存しない。TSF 側が仮想キーとシフト状態を解釈して、
//! この表現に落としてから渡す。
//!
//! SKK では「シフトを押しながらの英字」が見出し語と送り仮名の開始を示すため、
//! 大文字と小文字の区別そのものが意味を持つ。したがって英字はシフト修飾では
//! なく大文字・小文字の [`Key::Char`] として表す。

/// エンジンが解釈するキー。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// 文字キー。英字の大文字はシフト付きの打鍵を意味する。
    Char(char),
    /// 空白。変換の開始と次候補を兼ねる。
    Space,
    Enter,
    Backspace,
    Escape,
    Tab,
    /// 候補の前後移動。
    Up,
    Down,
    /// Ctrl 修飾付きの英字。英字は小文字で表す。
    Ctrl(char),
    /// 貼り付け (Ctrl+V・Shift+Insert)。貼る中身は打鍵に含まれないので、
    /// 受け取った側がクリップボードを読んで [`crate::Engine::paste`] へ渡す。
    Paste,
}

impl Key {
    /// 文字キーならその文字。
    pub fn as_char(self) -> Option<char> {
        match self {
            Self::Char(c) => Some(c),
            Self::Space => Some(' '),
            _ => None,
        }
    }

    /// 英大文字（＝シフト付きの英字打鍵）なら、その小文字。
    pub fn as_shifted_alpha(self) -> Option<char> {
        match self {
            Self::Char(c) if c.is_ascii_uppercase() => Some(c.to_ascii_lowercase()),
            _ => None,
        }
    }
}
