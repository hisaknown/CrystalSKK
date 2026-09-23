//! エクスプローラーで開く。
//!
//! サーバは隔離された入れ物の外にいる。TIP から直に開こうとすると、
//! ストアアプリの中では止められることがある。**開くのはこちらでやる。**

use std::io;
use std::path::Path;

use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

/// フォルダを開く。
pub fn open(folder: &Path) -> io::Result<()> {
    let target = HSTRING::from(folder.as_os_str());
    // SAFETY: どの文字列もこの呼び出しの間だけ生きていればよい。
    let result = unsafe { ShellExecuteW(None, w!("open"), &target, None, None, SW_SHOWNORMAL) };
    // 32 以下は失敗を表す (歴史的な取り決め)。
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "ShellExecute が {} を返しました",
            result.0 as isize
        )))
    }
}
