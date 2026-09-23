//! 確かめたり、知らせたりする小窓。
//!
//! 品書きから起こしたことの結果は、ここで言う。**黙って終わると、効いた
//! のかどうかが利用者に分からない。**

use windows::Win32::UI::WindowsAndMessaging::{
    IDOK, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_OKCANCEL,
    MB_SETFOREGROUND, MESSAGEBOX_STYLE, MessageBoxW,
};
use windows::core::{HSTRING, w};

use crate::menu;

/// 取り返しのつかないことをする前に確かめる。**既定の答えは「やめる」。**
///
/// Enter を打ち続けているうちに上書きしてしまわないように。
pub fn confirm(text: &str) -> bool {
    show(text, MB_OKCANCEL | MB_ICONWARNING | MB_DEFBUTTON2) == IDOK.0
}

/// したことを知らせる。
pub fn tell(text: &str) {
    show(text, MB_OK | MB_ICONINFORMATION);
}

/// できなかったことを知らせる。
pub fn complain(text: &str) {
    show(text, MB_OK | MB_ICONERROR);
}

fn show(text: &str, style: MESSAGEBOX_STYLE) -> i32 {
    // SAFETY: 文字列はこの呼び出しの間だけ生きていればよい。
    unsafe {
        MessageBoxW(
            Some(menu::owner()),
            &HSTRING::from(text),
            w!("CrystalSKK"),
            style | MB_SETFOREGROUND,
        )
        .0
    }
}
