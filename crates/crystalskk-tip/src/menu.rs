//! トレイのアイコンを右クリックしたときに出す品書き。
//!
//! 設定まわりの手当てをここから起こせるようにする。どれも**ファイルを
//! 触るのは辞書サーバ**で、こちらは頼むだけである (ADR-0016, ADR-0020)。
//!
//! 出し方は CorvusSKK に倣う。トレイの入力モード表示が右クリックされると
//! `OnClick` が呼ばれるので、そこで品書きを作って出す。古い言語バーは
//! `InitMenu` で項目を尋ねてくるので、同じ並びを渡す。

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::TextServices::{ITfMenu, TF_LBMENUF_SEPARATOR};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetForegroundWindow, MF_SEPARATOR, MF_STRING,
    TPM_LEFTALIGN, TPM_LEFTBUTTON, TPM_NONOTIFY, TPM_RETURNCMD, TPM_TOPALIGN, TPM_VERTICAL,
    TPMPARAMS, TrackPopupMenuEx,
};
use windows::core::{HSTRING, Result};

/// 品書きから選べること。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// 設定ファイルの置き場所を開く。
    OpenFolder,
    /// 設定を検査する。読めているか、キーとローマ字がぶつかっていないかを
    /// 窓に出す。ほかのアプリの TIP にも取り直させる (ADR-0040)。
    Validate,
    /// 設定ファイルを雛形で上書きする。
    ResetSettings,
    /// ローマ字テーブルを雛形で上書きする。
    ResetRomaji,
}

impl Command {
    /// 品書きでの番号。0 は「何も選ばなかった」に取ってあるので使わない。
    fn id(self) -> u32 {
        match self {
            Self::OpenFolder => 1,
            Self::Validate => 2,
            Self::ResetSettings => 3,
            Self::ResetRomaji => 4,
        }
    }

    /// 番号から引く。知らない番号なら `None`。
    pub fn from_id(id: u32) -> Option<Self> {
        ITEMS
            .iter()
            .flatten()
            .map(|(c, _)| *c)
            .find(|c| c.id() == id)
    }
}

/// 並び。`None` は区切り線。
///
/// 上書きの二つには「…」を付ける。**押すと確かめてくる**という印である。
pub const ITEMS: &[Option<(Command, &str)>] = &[
    Some((Command::OpenFolder, "設定フォルダを開く")),
    Some((Command::Validate, "設定を検査する")),
    None,
    Some((Command::ResetSettings, "設定ファイルを雛形で上書き…")),
    Some((Command::ResetRomaji, "ローマ字テーブルを雛形で上書き…")),
];

/// 右クリックされた場所に品書きを出し、選ばれたものを返す。
///
/// 何も選ばずに閉じられたら `None`。
pub fn pop_up(at: POINT, exclude: Option<RECT>) -> Option<Command> {
    // SAFETY: 品書きはこの関数の中で作って壊す。渡す構造体もこの中で
    // 用意したもので、呼び出しの間だけ生きていればよい。
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for item in ITEMS {
            let _ = match item {
                Some((command, text)) => AppendMenuW(
                    menu,
                    MF_STRING,
                    command.id() as usize,
                    &HSTRING::from(*text),
                ),
                None => AppendMenuW(menu, MF_SEPARATOR, 0, None),
            };
        }

        // 右クリックされた場所を覆わないように出す。
        let parameters = exclude.map(|area| TPMPARAMS {
            cbSize: size_of::<TPMPARAMS>() as u32,
            rcExclude: area,
        });
        let chosen = TrackPopupMenuEx(
            menu,
            (TPM_LEFTALIGN
                | TPM_TOPALIGN
                | TPM_NONOTIFY
                | TPM_RETURNCMD
                | TPM_LEFTBUTTON
                | TPM_VERTICAL)
                .0,
            at.x,
            at.y,
            owner(),
            parameters.as_ref().map(std::ptr::from_ref),
        );
        let _ = DestroyMenu(menu);
        u32::try_from(chosen.0).ok().and_then(Command::from_id)
    }
}

/// 古い言語バーに項目を渡す。選ばれると `OnMenuSelect` に番号が来る。
pub fn fill(menu: &ITfMenu) -> Result<()> {
    for item in ITEMS {
        let (id, flags, text) = match item {
            Some((command, text)) => (command.id(), 0, *text),
            None => (0, TF_LBMENUF_SEPARATOR, ""),
        };
        let text: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: 文字列はこの呼び出しの間だけ生きていればよい。
        unsafe {
            menu.AddMenuItem(
                id,
                flags,
                Default::default(),
                Default::default(),
                &text,
                std::ptr::null_mut(),
            )?;
        }
    }
    Ok(())
}

/// 品書きや確かめの窓の持ち主にする窓。
///
/// CorvusSKK は焦点のある窓を使う。無ければ前面の窓にする。**持ち主の無い
/// 窓は、アプリの後ろに隠れることがある。**
pub fn owner() -> HWND {
    // SAFETY: どちらも尋ねるだけで、何も変えない。
    unsafe {
        let focused = GetFocus();
        if focused.is_invalid() {
            GetForegroundWindow()
        } else {
            focused
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_comes_back_from_its_number() {
        for (command, _) in ITEMS.iter().flatten() {
            assert_eq!(Command::from_id(command.id()), Some(*command));
        }
    }

    #[test]
    fn choosing_nothing_is_not_a_command() {
        // 品書きを閉じただけのときは 0 が来る。**何かをしてはいけない。**
        assert_eq!(Command::from_id(0), None);
    }

    #[test]
    fn overwriting_asks_first() {
        for (command, text) in ITEMS.iter().flatten() {
            let overwrites = matches!(command, Command::ResetSettings | Command::ResetRomaji);
            assert_eq!(text.ends_with('…'), overwrites, "{text}");
        }
    }
}
