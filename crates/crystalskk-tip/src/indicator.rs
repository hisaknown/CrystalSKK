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

use std::cell::{Cell, RefCell};

use crystalskk_core::InputMode;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, DIB_RGB_COLORS, EndPaint, GetMonitorInfoW,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, PAINTSTRUCT, SetDIBitsToDevice,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_HWNDPARENT, GWLP_USERDATA,
    GetWindowLongPtrW, HWND_TOPMOST, IsWindowVisible, KillTimer, LWA_ALPHA, RegisterClassExW,
    SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetLayeredWindowAttributes, SetTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, UnregisterClassW, WINDOW_EX_STYLE, WM_DESTROY,
    WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::dpi;
use crate::guard::guard;
use crate::theme::Palette;
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
        if self.rounded.get() {
            popup::set_border(hwnd, palette.border);
        }
        // SAFETY: 窓は自分で作ったもの。描く中身は `self` にあり、窓より長く
        // 生きる (窓は `close` か `Drop` で壊す)。
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, std::ptr::from_ref(self) as isize);
            // 持ち主を入力先の窓にする。**持ち主の無い窓は、アプリの描画面の
            // 下に潜ることがある。** 候補の窓で一度はまった。
            if let Some(owner) = owner {
                SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner.0 as isize);
            }
        }

        // 窓を出すモニターの拡大率で描く (`crate::dpi`)。
        let dpi = dpi::at(POINT {
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
        let dpi = dpi::at(POINT {
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
                WINDOW_EX_STYLE(
                    WS_EX_NOACTIVATE.0
                        | WS_EX_TOOLWINDOW.0
                        | WS_EX_TOPMOST.0
                        // クリックを下へ通す。重ねた窓でないと効かない。
                        | WS_EX_LAYERED.0
                        | WS_EX_TRANSPARENT.0,
                ),
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
                // 重ねた窓は、濃さを決めるまで何も映らない。
                // SAFETY: 窓は直前に作ったもの。
                unsafe {
                    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
                }
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

/// 描く。地、縁、絵の色は `palette` に従う。`framed` なら縁を描く。
fn pixels(
    mode: Option<InputMode>,
    side: i32,
    palette: Palette,
    dpi: u32,
    framed: bool,
) -> Vec<u32> {
    let Palette {
        background,
        text: ink,
        border,
        ..
    } = palette;

    let side_u = side.max(0) as usize;
    let mut out = vec![background; side_u * side_u];
    for i in (0..side_u).filter(|_| framed) {
        out[i] = border;
        out[(side_u - 1) * side_u + i] = border;
        out[i * side_u] = border;
        out[i * side_u + side_u - 1] = border;
    }

    let glyph = u32::try_from(side - 2 * dpi::scale(PADDING, dpi)).unwrap_or(16);
    let (size, coverage) = icon::mode_coverage(mode, glyph);
    let size = size as usize;
    // 選ばれた絵が大きめでも、真ん中に置いてはみ出た分は切る。
    let offset = (side_u as isize - size as isize) / 2;
    for y in 0..size {
        for x in 0..size {
            let (tx, ty) = (x as isize + offset, y as isize + offset);
            if tx < 1 || ty < 1 || tx >= side_u as isize - 1 || ty >= side_u as isize - 1 {
                continue;
            }
            let alpha = u32::from(coverage[y * size + x]);
            let at = ty as usize * side_u + tx as usize;
            out[at] = blend(ink, out[at], alpha);
        }
    }
    out
}

/// 上に `alpha` の濃さで重ねる。
fn blend(over: u32, under: u32, alpha: u32) -> u32 {
    let channel = |shift: u32| {
        let o = (over >> shift) & 0xFF;
        let u = (under >> shift) & 0xFF;
        ((o * alpha + u * (255 - alpha)) / 255) << shift
    };
    channel(16) | channel(8) | channel(0)
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
                    let hdc = BeginPaint(hwnd, &mut ps);
                    let owner =
                        (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ModeWindow).as_ref();
                    if let Some((mode, palette, o_dpi, rounded)) = owner.and_then(|o| {
                        Some((
                            o.shown.get()?,
                            o.palette.get()?,
                            o.dpi.get(),
                            o.rounded.get(),
                        ))
                    }) {
                        let dpi = o_dpi;
                        let side = side(dpi);
                        let drawn = pixels(mode, side, palette, dpi, !rounded);
                        let info = BITMAPINFO {
                            bmiHeader: BITMAPINFOHEADER {
                                biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
                                biWidth: side,
                                // 負にすると上の行が先になる。
                                biHeight: -side,
                                biPlanes: 1,
                                biBitCount: 32,
                                biCompression: BI_RGB.0,
                                ..Default::default()
                            },
                            ..Default::default()
                        };
                        SetDIBitsToDevice(
                            hdc,
                            0,
                            0,
                            side as u32,
                            side as u32,
                            0,
                            0,
                            0,
                            side as u32,
                            drawn.as_ptr().cast(),
                            &info,
                            DIB_RGB_COLORS,
                        );
                    }
                    let _ = EndPaint(hwnd, &ps);
                }
                Ok(())
            });
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER => {
            // SAFETY: 自分の窓の時計を止めて隠す。
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER);
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
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
    fn colours_are_blended() {
        assert_eq!(blend(0xFF_FF_FF, 0x00_00_00, 255), 0xFF_FF_FF);
        assert_eq!(blend(0xFF_FF_FF, 0x00_00_00, 0), 0x00_00_00);
    }

    #[test]
    fn the_glyph_is_drawn_in_the_palette_inside_the_border() {
        let palette = Palette {
            background: 0x2B_2B_2B,
            text: 0xFF_FF_FF,
            border: 0x00_78_D4,
            selected_background: 0x00_78_D4,
            selected_text: 0xFF_FF_FF,
            key: 0x00_78_D4,
            backdrop_opacity: 100,
        };
        let side = 20;
        let drawn = pixels(Some(InputMode::Hiragana), side, palette, dpi::BASE, true);
        assert_eq!(drawn.len(), (side * side) as usize);
        assert_eq!(drawn[0], palette.border, "縁");
        let rounded = pixels(Some(InputMode::Hiragana), side, palette, dpi::BASE, false);
        assert_eq!(
            rounded[0], palette.background,
            "角を丸めたら縁は DWM に任せる"
        );
        assert_eq!(drawn[(side + 2) as usize], palette.background, "地");
        assert!(drawn.contains(&palette.text), "絵は文字の色で描かれる");
    }
}
