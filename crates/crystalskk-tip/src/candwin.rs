//! 候補の一覧を出す小窓。
//!
//! SKK の一覧は、他の日本語入力の候補ウィンドウとは出方が違う。何度か
//! 送って決まらないとき初めて開き、一度に出るのは選べる数だけで、選ぶのは
//! ラベルキーを押すことである。**その判断はすべてエンジンが済ませている**
//! ので、ここは渡された一ページを描くだけでよい。
//!
//! # 入力を受け取らない窓である
//!
//! 打鍵は TIP が受け取り、エンジンが解釈する。窓はそれを映すだけで、
//! 操作の相手ではない。だから**前面に出ても入力の焦点を奪ってはいけない**
//! (`WS_EX_NOACTIVATE`)。奪うと、打っている最中にアプリからカーソルが
//! 消える。
//!
//! # 絵柄は仮である
//!
//! 文字と枠を素朴に描くだけで、DPI とシステム色には従うが、暗い配色や
//! 注釈にはまだ対応していない。差し替える前提で、描き方は `paint` の
//! 一箇所に閉じてある。

use std::cell::RefCell;
use std::ffi::c_void;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_HIGHLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, CreateFontIndirectW,
    CreateSolidBrush, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
    EndPaint, FillRect, FrameRect, GetDC, GetDeviceCaps, GetSysColor, GetTextExtentPoint32W, HDC,
    HFONT, LOGPIXELSY, PAINTSTRUCT, ReleaseDC, SYS_COLOR_INDEX, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetSystemMetrics,
    GetWindowLongPtrW, HWND_TOPMOST, NONCLIENTMETRICSW, RegisterClassExW, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SPI_GETNONCLIENTMETRICS, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    SystemParametersInfoW, UnregisterClassW, WINDOW_EX_STYLE, WM_DESTROY, WM_PAINT, WNDCLASSEXW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::guard::guard;
use crate::log;

/// 一覧に出す一ページ。
#[derive(Debug, Default, Clone)]
pub struct Page {
    /// ラベルと、その候補の表示文字列。
    pub entries: Vec<(char, String)>,
    /// いま何ページ目か。1 から数える。
    pub number: usize,
    /// 全部で何ページか。
    pub count: usize,
}

impl Page {
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 窓に並べる行。最後の行はページの位置を示す。
    fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .entries
            .iter()
            .map(|(label, text)| format!("{label}: {text}"))
            .collect();
        if self.count > 1 {
            lines.push(format!("{} / {}", self.number, self.count));
        }
        lines
    }
}

/// 候補の一覧を出す小窓。
///
/// 一つの入力スレッドに一枚持つ。出すものが無くなったら隠すだけで、
/// 作り直さない。**打鍵のたびに窓を作っては壊すと、ちらつく。**
#[derive(Debug)]
pub struct CandidateWindow {
    hwnd: RefCell<HWND>,
}

impl Default for CandidateWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl CandidateWindow {
    pub fn new() -> Self {
        Self {
            hwnd: RefCell::new(HWND::default()),
        }
    }

    /// 一ページを出す。`anchor` は未確定の文字列の画面上の矩形。
    ///
    /// 出せなくても入力は続く。失敗は記録するだけにする。
    pub fn show(&self, page: &Page, anchor: RECT) {
        if page.is_empty() {
            self.hide();
            return;
        }
        let Some(hwnd) = self.ensure_window() else {
            return;
        };

        // 描く中身を窓に預ける。描画はいつ来るか分からないので、
        // 窓自身が持っていなければならない。
        let stored = Box::into_raw(Box::new(page.clone()));
        // SAFETY: 直前に作った箱を預け、前に預けていた分はここで落とす。
        unsafe {
            let previous = SetWindowLongPtrW(hwnd, GWLP_USERDATA, stored as isize);
            if previous != 0 {
                drop(Box::from_raw(previous as *mut Page));
            }
        }

        let (width, height) = measure(page);
        let (x, y) = place(anchor, width, height);
        // SAFETY: 窓は自分で作ったもの。
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE,
            );
            // 焦点は奪わない。打っている最中にカーソルが消えてはならない。
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }

    /// 窓を隠す。出ていなければ何もしない。
    pub fn hide(&self) {
        let hwnd = *self.hwnd.borrow();
        if hwnd.is_invalid() {
            return;
        }
        // SAFETY: 窓は自分で作ったもの。
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }

    /// 窓を壊す。無効化のときに呼ぶ。
    pub fn close(&self) {
        let hwnd = std::mem::take(&mut *self.hwnd.borrow_mut());
        if hwnd.is_invalid() {
            return;
        }
        // SAFETY: 窓は自分で作ったもの。預けた箱は `WM_DESTROY` で落ちる。
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    /// 窓を用意する。すでにあればそれを使う。
    fn ensure_window(&self) -> Option<HWND> {
        let existing = *self.hwnd.borrow();
        if !existing.is_invalid() {
            return Some(existing);
        }
        register_class()?;

        // SAFETY: 種別は直前に登録したもの。親を持たない浮いた窓を作る。
        let hwnd = unsafe {
            CreateWindowExW(
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
                log::write("候補の窓を作った");
                Some(hwnd)
            }
            Err(e) => {
                log::error(&format!("候補の窓を作れなかった: {}", e.message()));
                None
            }
        }
    }
}

impl Drop for CandidateWindow {
    fn drop(&mut self) {
        self.close();
    }
}

/// 窓の種別の名前。
const CLASS_NAME: PCWSTR = w!("CrystalSKKCandidates");

/// 窓の種別を登録する。一度でよい。
///
/// 登録はプロセスごとなので、同じプロセスに複数の入力スレッドがあっても
/// 一度で足りる。二度目は失敗するが、それは「すでにある」という意味なので
/// 気にしない。
fn register_class() -> Option<()> {
    use std::sync::OnceLock;
    static REGISTERED: OnceLock<bool> = OnceLock::new();

    let ok = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(0),
            lpfnWndProc: Some(window_proc),
            lpszClassName: CLASS_NAME,
            hInstance: crate::module().into(),
            ..Default::default()
        };
        // SAFETY: 名前も手続きもこのモジュールのもの。
        let atom = unsafe { RegisterClassExW(&class) };
        if atom == 0 {
            log::error("候補の窓の種別を登録できなかった");
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
            let _ = guard("候補の窓を描く", || {
                // SAFETY: 描画の手順どおり。預けた箱は窓が持っている。
                unsafe {
                    let mut ps = PAINTSTRUCT::default();
                    let hdc = BeginPaint(hwnd, &mut ps);
                    let stored = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Page;
                    if let Some(page) = stored.as_ref() {
                        paint(hdc, page);
                    }
                    let _ = EndPaint(hwnd, &ps);
                }
                Ok(())
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: 預けたのは自分の箱。二度落とさないよう 0 に戻す。
            unsafe {
                let stored = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if stored != 0 {
                    drop(Box::from_raw(stored as *mut Page));
                }
            }
            LRESULT(0)
        }
        // SAFETY: 既定の処理に委ねる。
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// 一覧を描く。**絵柄はここだけに閉じてある。**
///
/// # Safety
///
/// `hdc` が描画中のものであること。
unsafe fn paint(hdc: HDC, page: &Page) {
    // SAFETY: 呼び出し側の約束による。作ったものはこの関数の中で片付ける。
    unsafe {
        let font = ui_font();
        let previous = font.map(|f| SelectObject(hdc, f.into()));

        let (width, height) = measure(page);
        let area = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };

        let background = CreateSolidBrush(system_color(COLOR_WINDOW.0));
        FillRect(hdc, &area, background);
        let _ = DeleteObject(background.into());

        // 枠。地と同じ色では、背景に溶けて境目が分からない。
        let border = CreateSolidBrush(system_color(COLOR_HIGHLIGHT.0));
        FrameRect(hdc, &area, border);
        let _ = DeleteObject(border.into());

        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, system_color(COLOR_WINDOWTEXT.0));

        let line_height = line_height(hdc);
        for (index, line) in page.lines().iter().enumerate() {
            let top = PADDING + line_height * i32::try_from(index).unwrap_or(0);
            let mut rect = RECT {
                left: PADDING,
                top,
                right: width - PADDING,
                bottom: top + line_height,
            };
            let mut text: Vec<u16> = line.encode_utf16().collect();
            DrawTextW(
                hdc,
                &mut text,
                &mut rect,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            );
        }

        if let (Some(previous), Some(font)) = (previous, font) {
            SelectObject(hdc, previous);
            let _ = DeleteObject(font.into());
        }
    }
}

/// 窓の大きさを測る。
fn measure(page: &Page) -> (i32, i32) {
    // SAFETY: 画面の DC を借りて測り、すぐ返す。
    unsafe {
        let hdc = GetDC(None);
        let font = ui_font();
        let previous = font.map(|f| SelectObject(hdc, f.into()));

        let line_height = line_height(hdc);
        let mut widest = 0;
        let lines = page.lines();
        for line in &lines {
            let text: Vec<u16> = line.encode_utf16().collect();
            let mut size = SIZE::default();
            if GetTextExtentPoint32W(hdc, &text, &mut size).as_bool() {
                widest = widest.max(size.cx);
            }
        }

        if let (Some(previous), Some(font)) = (previous, font) {
            SelectObject(hdc, previous);
            let _ = DeleteObject(font.into());
        }
        ReleaseDC(None, hdc);

        let rows = i32::try_from(lines.len()).unwrap_or(1);
        (widest + PADDING * 2, line_height * rows + PADDING * 2)
    }
}

/// 一行の高さ。
///
/// # Safety
///
/// `hdc` に測りたい書体が選ばれていること。
unsafe fn line_height(hdc: HDC) -> i32 {
    // SAFETY: 呼び出し側の約束による。
    unsafe {
        let sample: Vec<u16> = "あA".encode_utf16().collect();
        let mut size = SIZE::default();
        if GetTextExtentPoint32W(hdc, &sample, &mut size).as_bool() {
            size.cy + scaled(LINE_GAP)
        } else {
            scaled(FALLBACK_LINE_HEIGHT)
        }
    }
}

/// 窓を置く場所を決める。
///
/// 未確定の文字列のすぐ下に出す。画面からはみ出すなら上へ回す。
/// **はみ出したまま出すと、肝心の候補が見えない。**
fn place(anchor: RECT, width: i32, height: i32) -> (i32, i32) {
    // SAFETY: 画面の大きさを尋ねるだけ。
    let (screen_left, screen_top, screen_width, screen_height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let screen_right = screen_left + screen_width;
    let screen_bottom = screen_top + screen_height;

    let mut x = anchor.left;
    let mut y = anchor.bottom;

    if x + width > screen_right {
        x = screen_right - width;
    }
    x = x.max(screen_left);

    if y + height > screen_bottom {
        // 下に入らないなら、未確定の文字列の上へ。
        y = anchor.top - height;
    }
    y = y.max(screen_top);

    (x, y)
}

/// 画面の案内に使う書体。
///
/// システムの設定に従う。自前で書体を選ぶと、利用者が大きさを変えていても
/// 追随できない。
fn ui_font() -> Option<HFONT> {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: u32::try_from(std::mem::size_of::<NONCLIENTMETRICSW>()).unwrap_or(0),
        ..Default::default()
    };
    // SAFETY: 大きさを正しく告げた構造体へ書かせる。
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some(std::ptr::from_mut(&mut metrics).cast::<c_void>()),
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    if !ok {
        return None;
    }
    // SAFETY: 受け取った書体の指定をそのまま使う。
    unsafe { CreateFontIndirectW(&metrics.lfMessageFont) }.into()
}

/// システムの色。
fn system_color(index: i32) -> COLORREF {
    // SAFETY: 番号を渡して色を受け取るだけ。
    COLORREF(unsafe { GetSysColor(SYS_COLOR_INDEX(index)) })
}

/// 拡大率に合わせて伸ばす。
fn scaled(value: i32) -> i32 {
    // SAFETY: 画面の DC を借りて問い合わせ、すぐ返す。
    let dpi = unsafe {
        let hdc = GetDC(None);
        let dpi = GetDeviceCaps(Some(hdc), LOGPIXELSY);
        ReleaseDC(None, hdc);
        dpi
    };
    let dpi = if dpi > 0 { dpi } else { BASE_DPI };
    value * dpi / BASE_DPI
}

/// 標準の拡大率での画素密度。
const BASE_DPI: i32 = 96;

/// 文字と枠の間。
const PADDING: i32 = 6;

/// 行と行の間。
const LINE_GAP: i32 = 4;

/// 書体を測れなかったときの一行の高さ。
const FALLBACK_LINE_HEIGHT: i32 = 20;

#[cfg(test)]
mod tests {
    use super::*;

    fn page(entries: &[(char, &str)], number: usize, count: usize) -> Page {
        Page {
            entries: entries
                .iter()
                .map(|(label, text)| (*label, (*text).to_owned()))
                .collect(),
            number,
            count,
        }
    }

    #[test]
    fn each_entry_becomes_a_line() {
        let page = page(&[('a', "漢字"), ('s', "感じ")], 1, 1);
        assert_eq!(page.lines(), vec!["a: 漢字", "s: 感じ"]);
    }

    #[test]
    fn the_page_number_shows_only_when_there_is_more_than_one() {
        let one = page(&[('a', "漢字")], 1, 1);
        assert_eq!(one.lines().len(), 1, "一ページしかないなら数えない");

        let many = page(&[('a', "漢字")], 2, 3);
        assert_eq!(many.lines().last().map(String::as_str), Some("2 / 3"));
    }

    #[test]
    fn an_empty_page_has_nothing_to_draw() {
        assert!(page(&[], 1, 1).is_empty());
    }

    #[test]
    fn a_window_that_would_fall_off_the_right_edge_is_pulled_back() {
        // 画面の大きさは環境によるので、右端の外に置こうとしたときに
        // 左へ寄ることだけを見る。
        let far_right = RECT {
            left: 1_000_000,
            top: 100,
            right: 1_000_100,
            bottom: 120,
        };
        let (x, _) = place(far_right, 200, 100);
        assert!(x < far_right.left, "画面の外へは出さない");
    }

    #[test]
    fn a_window_that_would_fall_below_goes_above_the_text() {
        let low = RECT {
            left: 10,
            top: 1_000_000,
            right: 100,
            bottom: 1_000_020,
        };
        let (_, y) = place(low, 200, 100);
        assert!(y < low.bottom, "下に入らないなら上へ回す");
    }
}
