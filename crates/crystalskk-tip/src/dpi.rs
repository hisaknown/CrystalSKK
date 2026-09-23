//! 小窓をどの拡大率で描くか。
//!
//! 拡大率はモニターごとに違いうる。**窓は、それを出すモニターの拡大率で
//! 描く。** 画面全体 (システム) の値で一律に描くと、拡大率の違うモニターに
//! 出たとき大きすぎたり小さすぎたりする。
//!
//! ただし、窓は入力先のアプリのプロセスの中で作られ、**そのアプリの拡大率の
//! 扱いに従う**。
//!
//! - モニターごとに対応するアプリ (Chrome など、最近のアプリの多く) では、
//!   座標は実際の画素で、窓はそのモニターの拡大率で描く。
//! - システムの値にだけ対応するアプリでは、座標はシステムの値で揃えられて
//!   いるので、システムの値で描く。モニターが違えば Windows が伸縮する。
//! - どちらにも対応しないアプリでは、100% として描き、Windows に伸ばして
//!   もらう。
//!
//! 取り違えると二重に伸びるので、アプリの扱いを見てから決める。

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, HFONT, MONITOR_DEFAULTTONEAREST, MonitorFromPoint,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_PER_MONITOR_AWARE, DPI_AWARENESS_SYSTEM_AWARE,
    GetAwarenessFromDpiAwarenessContext, GetDpiForMonitor, GetDpiForSystem,
    GetThreadDpiAwarenessContext, MDT_EFFECTIVE_DPI, SystemParametersInfoForDpi,
};
use windows::Win32::UI::WindowsAndMessaging::{NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS};

/// 100% のときの拡大率。
pub const BASE: u32 = 96;

/// 画面座標の `point` に窓を出すときの拡大率。
pub fn at(point: POINT) -> u32 {
    // SAFETY: どれも尋ねるだけ。
    unsafe {
        let awareness = GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext());
        if awareness == DPI_AWARENESS_PER_MONITOR_AWARE {
            let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
            let (mut x, mut y) = (0u32, 0u32);
            if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_ok() && x > 0 {
                return x;
            }
            GetDpiForSystem()
        } else if awareness == DPI_AWARENESS_SYSTEM_AWARE {
            GetDpiForSystem()
        } else {
            BASE
        }
    }
}

/// 100% のときの長さを、`dpi` の長さにする。四捨五入する。
pub fn scale(value: i32, dpi: u32) -> i32 {
    let dpi = i64::from(dpi);
    let scaled = (i64::from(value) * dpi + i64::from(BASE) / 2) / i64::from(BASE);
    i32::try_from(scaled).unwrap_or(value)
}

/// 案内に使う書体を、`dpi` の大きさで作る。
///
/// 書体はシステムの設定 (メッセージの書体) に従う。自前で選ぶと、利用者が
/// 大きさを変えていても追随できない。
pub fn message_font(dpi: u32) -> Option<HFONT> {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: u32::try_from(size_of::<NONCLIENTMETRICSW>()).unwrap_or(0),
        ..Default::default()
    };
    // SAFETY: 大きさを正しく告げた構造体へ書かせる。
    let ok = unsafe {
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            metrics.cbSize,
            Some(std::ptr::from_mut::<NONCLIENTMETRICSW>(&mut metrics).cast()),
            0,
            dpi,
        )
    }
    .is_ok();
    if !ok {
        return None;
    }
    // SAFETY: 受け取った書体の指定をそのまま使う。
    let font = unsafe { CreateFontIndirectW(&metrics.lfMessageFont) };
    (!font.is_invalid()).then_some(font)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_grow_with_the_scale() {
        assert_eq!(scale(16, 96), 16, "100%");
        assert_eq!(scale(16, 120), 20, "125%");
        assert_eq!(scale(16, 144), 24, "150%");
        assert_eq!(scale(6, 144), 9);
        assert_eq!(scale(5, 120), 6, "6.25 は 6");
        assert_eq!(scale(3, 120), 4, "3.75 は 4");
    }

    #[test]
    fn a_scale_can_be_found_anywhere() {
        let dpi = at(POINT { x: 0, y: 0 });
        assert!(dpi >= BASE, "{dpi}");
        assert!(message_font(dpi).is_some());
    }
}
