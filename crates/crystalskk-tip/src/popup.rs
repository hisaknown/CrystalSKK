//! アプリの上に浮かべる小窓 (候補の窓、モードの窓) に共通の見た目。
//!
//! Windows 11 のメニューや吹き出しに揃え、角を丸めて影を付ける。どちらも
//! DWM に頼むので、自分では描かない。

use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL, DwmSetWindowAttribute,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETDROPSHADOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::BOOL;

/// 窓の角を丸めるよう DWM に頼む。丸めてもらえたら `true`。
///
/// 小さめの丸みにする。**丸めた窓の枠は DWM が丸みに沿って描く**ので、
/// 自分では描かない。Windows 10 はこの頼みを知らないので、断られたら
/// 四角いまま自分で枠を描く。
pub fn round_corners(hwnd: HWND) -> bool {
    let preference = DWMWCP_ROUNDSMALL;
    // SAFETY: 渡す値はこの関数の変数で、大きさも渡している。
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const preference).cast(),
            u32::try_from(size_of::<DWM_WINDOW_CORNER_PREFERENCE>()).unwrap_or(4),
        )
    }
    .is_ok()
}

/// 角を丸めた窓の枠を DWM に描かせる。
///
/// 影が出るなら、それが窓の縁になるので枠は引かない。影を切っているときと、
/// ハイコントラストのときは `rgb` の色で引く。
pub fn set_border(hwnd: HWND, rgb: u32) {
    let color = if shadowed() && !crate::theme::high_contrast() {
        COLORREF(DWMWA_COLOR_NONE)
    } else {
        COLORREF(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF))
    };
    // SAFETY: 渡す値はこの関数の変数で、大きさも渡している。
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&raw const color).cast(),
            u32::try_from(size_of::<COLORREF>()).unwrap_or(4),
        );
    }
}

/// Windows が窓の下に影を出すか。「ウィンドウの下に影を表示する」を
/// 切っていれば、`CS_DROPSHADOW` を付けても影は出ない。
fn shadowed() -> bool {
    let mut on = BOOL::default();
    // SAFETY: 書き込み先はこの関数の変数。
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETDROPSHADOW,
            0,
            Some((&raw mut on).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    ok && on.as_bool()
}
