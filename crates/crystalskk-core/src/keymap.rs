//! キーの割り当て。
//!
//! SKK の操作 (確定、取り消し、前の候補、…) に名前を付け、それぞれに
//! キーを割り当てる。**割り当てるのは操作であって、状態ごとの振る舞い
//! ではない。** 同じ `q` が直接入力ではカタカナへの切り替え、見出し語
//! 入力ではカタカナでの確定になるように、状態による意味の違いはエンジンが
//! 引き受ける。ddskk の `skk-kakutei-key` や `skk-previous-candidate-char`
//! と同じ考え方である (ADR-0038)。
//!
//! 割り当てないキーもある。
//!
//! - シフトで見出し語や送り仮名を始めること。SKK の根幹で、キーではなく
//!   「大文字で打つ」ことに意味がある。
//! - Enter・Backspace・Escape・上下の矢印。どのアプリでも意味の決まって
//!   いるキーで、SKK はその意味に沿って使っているだけである。
//! - 候補の一覧から選ぶキーと、消してよいかの y/n。前者は
//!   `candidates.labels` で決まり、後者は問いへの答えである。
//!
//! 既定の割り当ては持たない (ADR-0020)。

use crate::key::Key;

/// 名前の付いた操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    /// 確定する。直接入力ではひらがなへ戻る (英数からかなへ戻る唯一の手段)。
    Kakutei,
    /// 取り消す。打ちかけを捨て、見出し語や候補を捨て、登録をやめる。
    Cancel,
    /// 変換を始める。候補選択では次の候補へ。
    StartHenkan,
    /// 前の候補へ。
    PreviousCandidate,
    /// いまの候補を辞書から消す。
    Purge,
    /// ひらがなとカタカナを切り替える。見出し語入力ではカタカナで確定する。
    ToggleKana,
    /// 半角カタカナへ切り替える。見出し語入力では半角カタカナで確定する。
    HalfKatakana,
    /// 半角英数へ。
    Ascii,
    /// 全角英数へ。
    FullAscii,
    /// 英字のまま見出し語を打つ (`/`)。
    Abbrev,
    /// 何も打たずに見出し語を始める (`Q`)。見出し語入力ではそこまでを確定して始め直す。
    SetHenkanPoint,
    /// 補完候補を順に選ぶ。
    Complete,
    /// 出ている補完候補を受け取り、変換して確定する。
    TakeCompletion,
    /// 接頭辞・接尾辞 (`>`)。
    Affix,
}

impl Command {
    /// すべての操作と、設定ファイルでの名前。
    pub const ALL: [(Self, &'static str); 14] = [
        (Self::Kakutei, "kakutei"),
        (Self::Cancel, "cancel"),
        (Self::StartHenkan, "start_henkan"),
        (Self::PreviousCandidate, "previous_candidate"),
        (Self::Purge, "purge"),
        (Self::ToggleKana, "toggle_kana"),
        (Self::HalfKatakana, "half_katakana"),
        (Self::Ascii, "ascii"),
        (Self::FullAscii, "full_ascii"),
        (Self::Abbrev, "abbrev"),
        (Self::SetHenkanPoint, "set_henkan_point"),
        (Self::Complete, "complete"),
        (Self::TakeCompletion, "take_completion"),
        (Self::Affix, "affix"),
    ];

    /// 直接入力か見出し語入力で、いつでも働く操作か。
    ///
    /// そうなら、割り当てた文字はそこでローマ字として読まれない。候補選択の
    /// 中でしか働かない操作や、補完候補が出ているときだけ働く操作は、
    /// ふだんの打鍵を奪わない。
    pub fn acts_while_typing(self) -> bool {
        !matches!(
            self,
            Self::PreviousCandidate | Self::Purge | Self::TakeCompletion
        )
    }

    /// 設定ファイルでの名前。
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(command, _)| *command == self)
            .map(|(_, name)| *name)
            .expect("すべての操作が ALL に並んでいる")
    }
}

/// キーから操作を引く表。
///
/// **一つのキーは一つの操作にしか割り当てない。** 読み込む側が確かめて
/// から作る。一つの操作に割り当てるキーは幾つでもよく、無くてもよい
/// (その操作は使わないということ)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    bindings: Vec<(Key, Command)>,
}

impl Keymap {
    /// 割り当ての並びから作る。同じキーが二度出てくれば、そのキーを返す。
    pub fn new(bindings: Vec<(Key, Command)>) -> Result<Self, Key> {
        for (at, (key, _)) in bindings.iter().enumerate() {
            if bindings[..at].iter().any(|(earlier, _)| earlier == key) {
                return Err(*key);
            }
        }
        Ok(Self { bindings })
    }

    /// このキーに割り当てた操作。
    pub fn command(&self, key: Key) -> Option<Command> {
        self.bindings
            .iter()
            .find(|(bound, _)| *bound == key)
            .map(|(_, command)| *command)
    }

    /// この操作に割り当てたキー。書かれた順。
    pub fn keys(&self, command: Command) -> impl Iterator<Item = Key> + '_ {
        self.bindings
            .iter()
            .filter(move |(_, bound)| *bound == command)
            .map(|(key, _)| *key)
    }

    /// この操作に割り当てた文字のキーのうち、最初のもの。窓で「このキーを
    /// 押す」と案内するのに使う。文字のキーが無ければ `None`。
    pub fn first_char(&self, command: Command) -> Option<char> {
        self.keys(command).find_map(|key| match key {
            Key::Char(c) => Some(c),
            _ => None,
        })
    }
}
