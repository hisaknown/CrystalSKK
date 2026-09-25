//! クリップボードの文字列を読む。
//!
//! 辞書登録の欄へ貼るためだけに使う。**ここはアプリのプロセスの中で
//! 動く**ので、読めなければ黙って空を返し、打鍵を止めない。

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use crate::log;

/// クリップボードにある文字列。無いか読めなければ `None`。
pub fn text() -> Option<String> {
    // SAFETY: 所有する窓なしで開く。閉じるのは下の `Opened` が引き受ける。
    if let Err(e) = unsafe { OpenClipboard(None) } {
        log::write(&format!("クリップボードを開けなかった: {}", e.message()));
        return None;
    }
    let _opened = Opened;

    // SAFETY: 開いている間だけ使う。持ち主はクリップボードで、解放しない。
    let handle: HANDLE = unsafe { GetClipboardData(u32::from(CF_UNICODETEXT.0)) }.ok()?;
    let global = HGLOBAL(handle.0);
    // SAFETY: 文字列の塊を錠をかけて覗き、読み終えたら外す。
    unsafe {
        let pointer = GlobalLock(global).cast::<u16>();
        if pointer.is_null() {
            return None;
        }
        // 終端の NUL を探す。塊の大きさを超えては読まない。
        let capacity = GlobalSize(global) / std::mem::size_of::<u16>();
        let units = std::slice::from_raw_parts(pointer, capacity);
        let length = units.iter().position(|&u| u == 0).unwrap_or(capacity);
        let text = String::from_utf16_lossy(&units[..length]);
        let _ = GlobalUnlock(global);
        Some(text)
    }
}

/// 開いたクリップボードを、どの道で抜けても閉じる。
struct Opened;

impl Drop for Opened {
    fn drop(&mut self) {
        // SAFETY: `OpenClipboard` が成功したときにだけ作られる。
        let _ = unsafe { CloseClipboard() };
    }
}
