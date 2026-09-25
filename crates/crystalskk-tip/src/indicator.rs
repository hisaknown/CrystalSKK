//! カーソルのそばに、いまの入力モードを短く出す窓。
//!
//! 出し方は CorvusSKK の「入力モード表示」に倣う (ADR-0025)。
//!
//! - カーソル (選択範囲) の真下に出す。下に入らなければ上へ回す。
//! - 決めた時間が経ったら消える。打鍵があったときや、入力先が変わった
//!   ときも消す。
//! - 絵はトレイと同じ入力モードの絵。色はアプリの明るさに合わせる
//!   ([`crate::theme::Palette`])。暗ければ黒い地に白い絵。
//! - **焦点を奪わず、クリックも受けない。** 一瞬出るだけの窓に、打鍵や
//!   クリックを取られてはならない。
//! - 候補の窓と同じく、角を丸めて地を透かし、Direct2D で描く
//!   ([`crate::popup`]、[`crate::draw`])。

use std::cell::{Cell, RefCell};

use crystalskk_core::InputMode;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_BITMAP_PROPERTIES,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_HWNDPARENT, GWLP_USERDATA,
    GetWindowLongPtrW, HTTRANSPARENT, HWND_TOPMOST, IsWindowVisible, KillTimer, RegisterClassExW,
    SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetTimer, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, UnregisterClassW, WINDOW_EX_STYLE, WM_DESTROY, WM_NCACTIVATE, WM_NCHITTEST,
    WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::guard::guard;
use crate::theme::Palette;
use crate::{dpi, draw};
use crate::{icon, log, popup};

/// 絵の大きさ。100% のときの画素数。拡大率に合わせて伸ばす。
///
/// トレイと同じ 16 画素にする。**打っている文字より目立っては困る。**
const GLYPH: i32 = 16;

/// 絵のまわりの余白。角の丸みに絵が食われないだけ空ける。
const PADDING: i32 = 5;

/// カーソルとのあいだ。
const GAP: i32 = 2;

/// 消えるまでの時計の番号。
const TIMER: usize = 1;

/// カーソルのそばに出す、入力モードの窓。
#[derive(Debug, Default)]
pub struct ModeWindow {
    hwnd: RefCell<HWND>,
    /// いま出している絵。窓の手続きが描くときに読む。
    shown: Cell<Option<Option<InputMode>>>,
    /// 描く色。出すときに受け取る。
    palette: Cell<Option<Palette>>,
    /// 描く拡大率。出すときに、出すモニターで決める。
    dpi: Cell<u32>,
    /// 角を DWM が丸めているか。丸めているなら枠も DWM が描く。
    rounded: Cell<bool>,
    /// 地を DWM に透かさせているか。
    backdrop: Cell<bool>,
}

impl ModeWindow {
    pub fn new() -> Self {
        Self::default()
    }

    /// `caret` (画面座標) の真下に `mode` を出し、`duration_ms` 後に消す。
    /// `None` は入力方式が切。
    pub fn show(
        &self,
        mode: Option<InputMode>,
        caret: RECT,
        owner: Option<HWND>,
        duration_ms: u32,
        palette: Palette,
    ) {
        let Some(hwnd) = self.ensure_window() else {
            return;
        };
        self.shown.set(Some(mode));
        self.palette.set(Some(palette));
        let rounded = self.rounded.get();
        if rounded {
            popup::set_border(hwnd, palette.border);
        }
        // 地を透かすのは、角を丸められる Windows 11 のときだけ。
        self.backdrop
            .set(rounded && popup::set_backdrop(hwnd, popup::is_dark(palette.background)));
        // SAFETY: 窓は自分で作ったもの。描く中身は `self` にあり、窓より長く
        // 生きる (窓は `close` か `Drop` で壊す)。
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, std::ptr::from_ref(self) as _);
            // 持ち主を入力先の窓にする。**持ち主の無い窓は、アプリの描画面の
            // 下に潜ることがある。** 候補の窓で一度はまった。
            if let Some(owner) = owner {
                SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner.0 as _);
            }
        }

        // 窓を出すモニターの拡大率で描く (`crate::dpi`)。
        let dpi = draw::dpi_at(POINT {
            x: caret.left,
            y: caret.bottom,
        });
        self.dpi.set(dpi);
        let side = side(dpi);
        let (x, y) = place(caret, side, side, work_area(caret), dpi::scale(GAP, dpi));
        // SAFETY: 窓は自分で作ったもの。
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, true);
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, side, side, SWP_NOACTIVATE);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            if self.backdrop.get() {
                popup::keep_lit(hwnd);
            }
            // 出し直すたびに時計を掛け直す。
            SetTimer(Some(hwnd), TIMER, duration_ms, None);
        }
    }

    /// 窓が出ているか。
    pub fn is_visible(&self) -> bool {
        let hwnd = *self.hwnd.borrow();
        // SAFETY: 尋ねるだけ。
        !hwnd.is_invalid() && unsafe { IsWindowVisible(hwnd) }.as_bool()
    }

    /// 絵も消える時刻もそのままに、`caret` のそばへ動かす。
    pub fn follow(&self, caret: RECT) {
        if !self.is_visible() {
            return;
        }
        let hwnd = *self.hwnd.borrow();
        let dpi = draw::dpi_at(POINT {
            x: caret.left,
            y: caret.bottom,
        });
        let rescaled = self.dpi.replace(dpi) != dpi;
        let side = side(dpi);
        let (x, y) = place(caret, side, side, work_area(caret), dpi::scale(GAP, dpi));
        // SAFETY: 窓は自分で作ったもの。
        unsafe {
            if rescaled {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, true);
            }
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, side, side, SWP_NOACTIVATE);
        }
    }

    /// 消す。出ていなければ何もしない。
    pub fn hide(&self) {
        let hwnd = *self.hwnd.borrow();
        if hwnd.is_invalid() {
            return;
        }
        // SAFETY: 窓は自分で作ったもの。
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER);
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }

    /// 窓を壊す。無効化のときに呼ぶ。
    pub fn close(&self) {
        let hwnd = std::mem::take(&mut *self.hwnd.borrow_mut());
        if hwnd.is_invalid() {
            return;
        }
        // SAFETY: 自分で作った窓を壊す。預けた指し先は外してから。
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(hwnd);
        }
    }

    fn ensure_window(&self) -> Option<HWND> {
        let existing = *self.hwnd.borrow();
        if !existing.is_invalid() {
            return Some(existing);
        }
        register_class()?;
        // SAFETY: 種別は直前に登録したもの。
        let hwnd = unsafe {
            CreateWindowExW(
                // 重ねた窓 (`WS_EX_LAYERED`) にはしない。DWM が地を透かさない。
                // クリックは `WM_NCHITTEST` で下へ通す。
                WINDOW_EX_STYLE(WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0),
                CLASS_NAME,
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            )
        };
        match hwnd {
            Ok(hwnd) => {
                *self.hwnd.borrow_mut() = hwnd;
                self.rounded.set(popup::round_corners(hwnd));
                Some(hwnd)
            }
            Err(e) => {
                log::error(&format!("モードの窓を作れなかった: {}", e.message()));
                None
            }
        }
    }
}

impl Drop for ModeWindow {
    fn drop(&mut self) {
        self.close();
    }
}

/// 窓の一辺。絵と、そのまわりの余白。
fn side(dpi: u32) -> i32 {
    dpi::scale(GLYPH, dpi) + dpi::scale(PADDING, dpi) * 2
}

/// 置き場所を決める。
///
/// カーソルの真下、左端を揃える。作業領域 (タスクバーを除いた画面) から
/// はみ出すなら押し戻し、下に入らなければカーソルの上へ回す。
fn place(caret: RECT, width: i32, height: i32, work: RECT, gap: i32) -> (i32, i32) {
    let x = caret.left.min(work.right - width).max(work.left);
    let below = caret.bottom + gap;
    let y = if below + height <= work.bottom {
        below
    } else {
        caret.top - gap - height
    };
    (x, y.max(work.top))
}

/// カーソルのある画面の作業領域。
fn work_area(caret: RECT) -> RECT {
    let point = POINT {
        x: caret.left,
        y: caret.bottom,
    };
    // SAFETY: 尋ねるだけ。
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: u32::try_from(size_of::<MONITORINFO>()).unwrap_or(0),
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            info.rcWork
        } else {
            RECT {
                left: i32::MIN / 2,
                top: i32::MIN / 2,
                right: i32::MAX / 2,
                bottom: i32::MAX / 2,
            }
        }
    }
}

/// 描く。地、縁、絵の色は `palette` に従う。
///
/// 絵は窓の真ん中に、拡大率に合った大きさのものを画素そのままで置く。
/// **伸び縮みさせると滲む。**
fn paint(hwnd: HWND, window: &ModeWindow) {
    let (Some(mode), Some(palette)) = (window.shown.get(), window.palette.get()) else {
        return;
    };
    let dpi = window.dpi.get();
    let (rounded, backdrop) = (window.rounded.get(), window.backdrop.get());
    #[allow(clippy::cast_precision_loss)]
    let (side, scale) = (
        side(dpi) as f32 * dpi::BASE as f32 / dpi as f32,
        dpi::BASE as f32 / dpi as f32,
    );
    let glyph = u32::try_from(dpi::scale(GLYPH, dpi)).unwrap_or(16);
    let (size, coverage) = icon::mode_coverage(mode, glyph);
    let tinted = premultiplied(coverage, palette.text);

    draw::with_target(hwnd, dpi, |target| {
        let ground = draw::ground(palette.background, backdrop, palette.backdrop_opacity);
        // SAFETY: 描いている最中の描く先を塗りつぶすだけ。
        unsafe { target.Clear(Some(&ground)) };

        // 縁。角を丸めているなら DWM が描く (`crate::popup`)。
        if !rounded
            // SAFETY: 描いている最中の描く先に、筆を作らせるだけ。
            && let Ok(border) =
                unsafe { target.CreateSolidColorBrush(&draw::color(palette.border), None) }
        {
            let half = scale / 2.0;
            let edge = draw::rect(half, half, side - half, side - half);
            // SAFETY: 描いている最中の描く先に、線を引かせるだけ。
            unsafe { target.DrawRectangle(&edge, &border, scale, None) };
        }

        let properties = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 0.0,
            dpiY: 0.0,
        };
        let pixels = D2D_SIZE_U {
            width: size,
            height: size,
        };
        // SAFETY: 絵の画素は `size` 四方あり、一行の長さも渡している。
        let Ok(bitmap) = (unsafe {
            target.CreateBitmap(pixels, Some(tinted.as_ptr().cast()), size * 4, &properties)
        }) else {
            return;
        };
        // 画素そのままの大きさで真ん中に置く。窓より大きければ、はみ出た分は切れる。
        #[allow(clippy::cast_precision_loss)]
        let extent = size as f32 * scale;
        let at = ((side - extent) / 2.0 / scale).round() * scale;
        let place = draw::rect(at, at, at + extent, at + extent);
        // SAFETY: 描いている最中の描く先に、絵を置かせるだけ。
        unsafe {
            target.DrawBitmap(
                &bitmap,
                Some(&place),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
            );
        }
    });
}

/// 濃さに色を付け、Direct2D の画素 (`0xAARRGGBB`、色は濃さを掛けた値) にする。
fn premultiplied(coverage: &[u8], ink: u32) -> Vec<u32> {
    let channel = |shift: u32, alpha: u32| ((((ink >> shift) & 0xFF) * alpha + 127) / 255) << shift;
    coverage
        .iter()
        .map(|alpha| {
            let alpha = u32::from(*alpha);
            (alpha << 24) | channel(16, alpha) | channel(8, alpha) | channel(0, alpha)
        })
        .collect()
}

const CLASS_NAME: PCWSTR = w!("CrystalSKKModeIndicator");

fn register_class() -> Option<()> {
    use std::sync::OnceLock;
    static REGISTERED: OnceLock<bool> = OnceLock::new();
    let ok = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or(0),
            lpfnWndProc: Some(window_proc),
            lpszClassName: CLASS_NAME,
            hInstance: crate::module().into(),
            // 影を付ける。候補の窓と揃える。
            style: CS_DROPSHADOW,
            ..Default::default()
        };
        // SAFETY: 名前も手続きもこのモジュールのもの。
        let atom = unsafe { RegisterClassExW(&class) };
        if atom == 0 {
            log::error("モードの窓の種別を登録できなかった");
            return false;
        }
        true
    });
    ok.then_some(())
}

/// 種別の登録を外す。DLL が降ろされるときに呼ぶ。
pub fn unregister_class() {
    // SAFETY: 登録していなければ失敗するだけで、害はない。
    unsafe {
        let _ = UnregisterClassW(CLASS_NAME, Some(crate::module().into()));
    }
}

/// 窓の手続き。
///
/// # Safety
///
/// Windows から呼ばれる。引数は Windows が用意したもの。
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            // パニックを窓の手続きから外へ出すと、アプリごと落ちる。
            let _ = guard("モードの窓を描く", || {
                // SAFETY: 描画の手順どおり。預けた指し先は、生きている間だけ入っている。
                unsafe {
                    let mut ps = PAINTSTRUCT::default();
                    let _ = BeginPaint(hwnd, &mut ps);
                    let owner =
                        (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ModeWindow).as_ref();
                    if let Some(owner) = owner {
                        paint(hwnd, owner);
                    }
                    let _ = EndPaint(hwnd, &ps);
                }
                Ok(())
            });
            LRESULT(0)
        }
        // クリックは下の窓へ通す。
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        // 透かした地を保つため、いつも前面にいるものとして扱わせる。
        WM_NCACTIVATE => {
            // SAFETY: 前面かどうかだけを差し替えて、既定の処理に委ねる。
            unsafe { DefWindowProcW(hwnd, message, WPARAM(1), lparam) }
        }
        WM_TIMER if wparam.0 == TIMER => {
            // SAFETY: 自分の窓の時計を止めて隠す。
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER);
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            draw::forget(hwnd);
            LRESULT(0)
        }
        // SAFETY: 既定の処理に委ねる。
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: RECT = RECT {
        left: 0,
        top: 0,
        right: 1000,
        bottom: 800,
    };

    fn caret(x: i32, y: i32) -> RECT {
        RECT {
            left: x,
            top: y,
            right: x + 1,
            bottom: y + 20,
        }
    }

    #[test]
    fn it_sits_just_below_the_caret() {
        assert_eq!(place(caret(100, 200), 32, 32, WORK, 2), (100, 222));
    }

    #[test]
    fn it_goes_above_when_there_is_no_room_below() {
        // カーソルの下端は 790。下に 32 は入らない。
        assert_eq!(place(caret(100, 770), 32, 32, WORK, 2), (100, 736));
    }

    #[test]
    fn it_stays_inside_the_work_area_sideways() {
        assert_eq!(place(caret(990, 200), 32, 32, WORK, 2).0, 968);
        assert_eq!(place(caret(-50, 200), 32, 32, WORK, 2).0, 0);
    }

    #[test]
    fn the_glyph_is_inked_with_its_coverage() {
        let drawn = premultiplied(&[0, 255, 128], 0xFF_80_00);
        assert_eq!(drawn[0], 0, "濃さの無いところは透明");
        assert_eq!(drawn[1], 0xFF_FF_80_00, "濃いところは色そのまま");
        assert_eq!(drawn[2], 0x80_80_40_00, "半分の濃さなら色も半分");
    }
}
