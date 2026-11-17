//! アプリの上に浮かべる小窓 (候補の窓、モードの窓) に共通の見た目。
//!
//! Windows 11 のメニューや吹き出しに揃え、角を丸めて影を付ける。どちらも
//! DWM に頼むので、自分では描かない。

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW,
    DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETDROPSHADOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SendMessageW, SystemParametersInfoW,
    WM_NCACTIVATE,
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

/// 窓の地を透かすか (Acrylic)。透かせたら `true`。
///
/// 地の透け方と色合いは Windows が決める。**Windows 11 のメニューや吹き出しと
/// 同じ `DWMSBT_TRANSIENTWINDOW` にし**、明暗は `dark` で教える。透かすには、
/// 窓の中身を透明にしたうえで、DWM の縁を中身いっぱいに広げる。
///
/// 次のときは透かさず、これまでどおり地の色で塗る。
///
/// - 「透明効果」を切っているとき。利用者が透けるのを嫌っている。
/// - ハイコントラストのとき。配色を上書きしない。
/// - Windows がこの頼みを知らないとき (Windows 10、Windows 11 の 22H2 より前)。
pub fn set_backdrop(hwnd: HWND, dark: bool) -> bool {
    let wanted = crate::theme::transparency() && !crate::theme::high_contrast();
    let kind = if wanted {
        DWMSBT_TRANSIENTWINDOW
    } else {
        DWMSBT_NONE
    };
    let dark = BOOL::from(dark);
    // SAFETY: 渡す値はこの関数の変数で、大きさも渡している。
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark).cast(),
            u32::try_from(size_of::<BOOL>()).unwrap_or(4),
        );
        let set = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&raw const kind).cast(),
            u32::try_from(size_of::<DWM_SYSTEMBACKDROP_TYPE>()).unwrap_or(4),
        )
        .is_ok();
        let on = wanted && set;
        // -1 は「中身いっぱい」。0 で元に戻る。
        let inset = if on { -1 } else { 0 };
        let margins = MARGINS {
            cxLeftWidth: inset,
            cxRightWidth: inset,
            cyTopHeight: inset,
            cyBottomHeight: inset,
        };
        let extended = DwmExtendFrameIntoClientArea(hwnd, &margins).is_ok();
        on && extended
    }
}

/// 透かした地を保つ。
///
/// 透かした地は、窓が前面でないと単色に落ちる。小窓は焦点を奪えないので、
/// 前面にいるものとして扱わせる。窓の手続きのほうでも、`WM_NCACTIVATE` を
/// いつも前面として既定の処理に渡すこと。
pub fn keep_lit(hwnd: HWND) {
    // SAFETY: 自分の窓に知らせを送るだけ。
    unsafe {
        let _ = SendMessageW(hwnd, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0)));
    }
}

/// 地の色が暗いか。透かしたときの色合いを、塗るはずだった地に揃える。
pub fn is_dark(rgb: u32) -> bool {
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    r * 299 + g * 587 + b * 114 < 128 * 1000
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn darkness_follows_the_ground() {
        assert!(is_dark(0x2B_2B_2B));
        assert!(!is_dark(0xFF_FF_FF));
        assert!(!is_dark(0xF3_F3_F3));
    }
}
