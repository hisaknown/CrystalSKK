//! 候補の一覧を出す小窓。
//!
//! SKK の一覧は、他の日本語入力の候補ウィンドウとは出方が違う。何度か
//! 送って決まらないとき初めて開き、一度に出るのは選べる数だけで、選ぶのは
//! ラベルキーを押すことである。**その判断はすべてエンジンが済ませている**
//! ので、ここは渡された一ページを描くだけでよい。
//!
//! # アプリの窓を親にする
//!
//! 作るときに入力先アプリの窓を親 (オーナー) として渡す。**親のない
//! ポップアップは、アプリの描画面の下に潜って見えないことがある。**
//! ストアアプリがまさにそうで、窓は出来ているのに何も見えなかった。
//!
//! 親を持てば、その窓の上に重なり、アプリが閉じれば一緒に片付く。
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
    BeginPaint, CreateFontIndirectW, CreateSolidBrush, DT_CALCRECT, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DeleteObject, DrawTextW, EndPaint,
    FillRect, FrameRect, GetDC, GetDeviceCaps, GetTextExtentPoint32W, HDC, HFONT, InvalidateRect,
    LOGPIXELSY, PAINTSTRUCT, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_HWNDPARENT,
    GWLP_USERDATA, GetSystemMetrics, GetWindowLongPtrW, HWND_TOPMOST, NONCLIENTMETRICSW,
    RegisterClassExW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SPI_GETNONCLIENTMETRICS, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, SystemParametersInfoW, UnregisterClassW, WINDOW_EX_STYLE, WM_DESTROY,
    WM_PAINT, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::guard::guard;
use crate::log;
use crate::theme::Palette;

/// 窓に出すもの。
///
/// 候補の一覧と辞書登録は、**同じ一枚の窓を使い分ける**。どちらも
/// 「いま入力している場所のそばに出す、数行の案内」であって、二枚に
/// 分ける理由がない。CorvusSKK も同じ窓に出している。
#[derive(Debug, Clone)]
pub enum Content {
    /// 候補の一覧。
    Page(Page),
    /// 辞書登録の入力欄。
    Registration(Registration),
    /// いま選んでいる補完候補。
    Completion(Completion),
    /// 伝えたいこと。**黙って違う結果を出すより、言うほうがよい。**
    Notice(String),
    /// 候補を一つずつ見せているあいだの、その候補の注釈。
    ///
    /// 注釈は入力欄には出さない。**長いことが多く、打っている場所に
    /// 出るとうっとうしい。** 窓の幅で折り返して、全文を出す。
    Annotation(String),
}

impl Content {
    /// 窓に並べる行。注釈だけの窓は折り返すので、ここでは一行として返す。
    fn lines(&self) -> Vec<Line> {
        match self {
            Self::Page(page) => page.lines(),
            Self::Registration(registration) => registration.lines(),
            Self::Completion(completion) => completion.lines(),
            Self::Notice(text) => vec![Line::plain(format!("[{text}]"))],
            Self::Annotation(text) => vec![Line::plain(text.clone())],
        }
    }

    /// 折り返して見せる中身か。
    fn wraps(&self) -> bool {
        matches!(self, Self::Annotation(_))
    }

    /// 反転して見せる行。無ければ `None`。
    ///
    /// **記号で示すより、地と文字の色を入れ替えるほうがよい。** 記号は
    /// フォントによって幅も形も変わるし、語そのものと紛れる。
    fn highlight(&self) -> Option<usize> {
        match self {
            Self::Completion(completion) if completion.taken => Some(completion.current),
            _ => None,
        }
    }

    fn is_empty(&self) -> bool {
        self.lines().is_empty()
    }
}

/// 窓の一行。本文と、その右に薄く添える注釈。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub note: Option<String>,
}

impl Line {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            note: None,
        }
    }
}

impl PartialEq<&str> for Line {
    fn eq(&self, other: &&str) -> bool {
        self.note.is_none() && self.text == *other
    }
}

/// いま選んでいる補完候補。
///
/// 出すのは**変換先**である。読みは見れば大抵分かるので、窓に出して意味が
/// あるのは変換した後の姿のほうである。
///
/// 動的補完のあいだは一行しか出ない。出る候補は一つきりで、選ぶ操作が
/// 無いからである (ADR-0019)。Tab で巡り始めたら前後を並べる。
#[derive(Debug, Default, Clone)]
pub struct Completion {
    /// このページに出す変換先。
    pub entries: Vec<String>,
    /// ページの中での、選んでいる候補の位置。
    pub current: usize,
    /// もう受け取ったものか。
    pub taken: bool,
    /// 補完候補を受け取るキー。受け取る前の一行に出す。
    pub take_key: char,
    /// いま何ページ目か。1 から数える。
    pub number: usize,
    /// 全部で何ページか。
    pub count: usize,
}

impl Completion {
    fn lines(&self) -> Vec<Line> {
        if !self.taken {
            // 一覧と同じ「キー: 語」の形にする。押すキーがそのまま左に出る。
            return self
                .entries
                .first()
                .map(|word| Line::plain(format!("{}: {word}", self.take_key)))
                .into_iter()
                .collect();
        }

        // 受け取った後は打鍵の案内を出さない。**同じキーが同じことを
        // しないのに、出したままにはできない。** 選んでいる候補は
        // [`Content::highlight`] が反転させる。
        let mut lines: Vec<Line> = self.entries.iter().map(Line::plain).collect();
        if self.count > 1 {
            lines.push(Line::plain(format!("{} / {}", self.number, self.count)));
        }
        lines
    }
}

/// 辞書登録の様子。
#[derive(Debug, Default, Clone)]
pub struct Registration {
    /// 登録しようとしている見出し語。送り仮名があれば含める。
    pub key: String,
    /// これまでに溜まった語と、いま入力中の文字列を繋げたもの。
    pub text: String,
    /// 積まれている枠の数。入れ子の深さを括弧の数で示す。
    pub depth: usize,
}

impl Registration {
    fn lines(&self) -> Vec<Line> {
        // 入れ子の深さを括弧の数で示す。CorvusSKK と同じ見せ方で、
        // **登録の中で登録が始まったことが一目で分かる。**
        let open = "[".repeat(self.depth.max(1));
        let close = "]".repeat(self.depth.max(1));
        // 文字の入る場所を示す印。窓には本物のカーソルが無い。
        vec![Line::plain(format!(
            "{open}登録{close} {}: {}│",
            self.key, self.text
        ))]
    }
}

/// 一覧に出す一ページ。
#[derive(Debug, Default, Clone)]
pub struct Page {
    /// ラベルと、その候補の表示文字列。
    pub entries: Vec<(char, String)>,
    /// それぞれの候補の注釈。`entries` と同じ順。注釈を出さないなら空。
    pub notes: Vec<Option<String>>,
    /// いま何ページ目か。1 から数える。
    pub number: usize,
    /// 全部で何ページか。
    pub count: usize,
}

impl Page {
    /// 窓に並べる行。最後の行はページの位置を示す。
    fn lines(&self) -> Vec<Line> {
        let mut lines: Vec<Line> = self
            .entries
            .iter()
            .enumerate()
            .map(|(at, (label, text))| Line {
                text: format!("{label}: {text}"),
                note: self.notes.get(at).cloned().flatten(),
            })
            .collect();
        if self.count > 1 {
            lines.push(Line::plain(format!("{} / {}", self.number, self.count)));
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

    /// 窓に一つ出す。`anchor` は未確定の文字列の画面上の矩形。
    ///
    /// 出せなくても入力は続く。失敗は記録するだけにする。
    pub fn show(&self, content: &Content, anchor: RECT, owner: Option<HWND>, palette: Palette) {
        if content.is_empty() {
            self.hide();
            return;
        }
        let Some(hwnd) = self.ensure_window(owner) else {
            return;
        };
        self.follow_owner(hwnd, owner);

        // 描く中身を窓に預ける。描画はいつ来るか分からないので、
        // 窓自身が持っていなければならない。
        let stored = Box::into_raw(Box::new(Painted {
            content: content.clone(),
            palette,
        }));
        // SAFETY: 直前に作った箱を預け、前に預けていた分はここで落とす。
        unsafe {
            let previous = SetWindowLongPtrW(hwnd, GWLP_USERDATA, stored as isize);
            if previous != 0 {
                drop(Box::from_raw(previous as *mut Painted));
            }
        }

        let (width, height) = measure(content);
        let (x, y) = place(anchor, width, height);
        // 中身が変われば描き直す。大きさが同じままでも中身は違いうるので、
        // 動かしただけで描き直されるとは限らない。
        // SAFETY: 窓は自分で作ったもの。
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
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

    /// 親が変わっていたら付け替える。
    ///
    /// 入力先が別の窓へ移れば、重なる先もそちらへ移さなければならない。
    fn follow_owner(&self, hwnd: HWND, owner: Option<HWND>) {
        let Some(owner) = owner else {
            return;
        };
        // SAFETY: どちらも有効な窓。親の付け替えは Windows が認めている。
        unsafe {
            let current = GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT);
            if current != owner.0 as isize {
                SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner.0 as isize);
            }
        }
    }

    /// 窓を用意する。すでにあればそれを使う。
    fn ensure_window(&self, owner: Option<HWND>) -> Option<HWND> {
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
                // 親。ここが `None` だと、アプリの描画面の下に潜りうる。
                owner,
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
            // 大きさが変わったら全部描き直す。これが無いと、**縮んだとき
            // 古い絵がそのまま残る**。Windows は新たに現れた部分しか
            // 描き直さないため、七件の一覧から五件の一覧へ移ると、
            // 前のページの五件が居座って見える。
            style: CS_HREDRAW | CS_VREDRAW,
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
                    let stored = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Painted;
                    if let Some(painted) = stored.as_ref() {
                        paint(hdc, &painted.content, painted.palette);
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
                    drop(Box::from_raw(stored as *mut Painted));
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
unsafe fn paint(hdc: HDC, content: &Content, palette: Palette) {
    // SAFETY: 呼び出し側の約束による。作ったものはこの関数の中で片付ける。
    unsafe {
        let font = ui_font();
        let previous = font.map(|f| SelectObject(hdc, f.into()));

        let (width, height) = measure(content);
        let area = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };

        let background = CreateSolidBrush(colorref(palette.background));
        FillRect(hdc, &area, background);
        let _ = DeleteObject(background.into());

        // 枠。地と同じ色では、背景に溶けて境目が分からない。
        let border = CreateSolidBrush(colorref(palette.border));
        FrameRect(hdc, &area, border);
        let _ = DeleteObject(border.into());

        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, colorref(palette.text));

        // 注釈だけの窓は、窓の幅で折り返して全文を出す。
        if content.wraps() {
            let text = content
                .lines()
                .into_iter()
                .next()
                .map(|line| line.text)
                .unwrap_or_default();
            let mut rect = RECT {
                left: PADDING,
                top: PADDING,
                right: width - PADDING,
                bottom: height - PADDING,
            };
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            DrawTextW(
                hdc,
                &mut wide,
                &mut rect,
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
            );
        } else {
            let line_height = line_height(hdc);
            let highlight = content.highlight();
            for (index, line) in content.lines().iter().enumerate() {
                let top = PADDING + line_height * i32::try_from(index).unwrap_or(0);
                let rect = RECT {
                    left: PADDING,
                    top,
                    right: width - PADDING,
                    bottom: top + line_height,
                };

                // 選んでいる行は地と文字の色を入れ替える。**記号で示すより
                // 確かで、フォントによって見た目が変わらない。**
                //
                // 帯は余白いっぱいまで広げる。文字の幅だけ塗ると、行によって
                // 帯の長さが変わってちらついて見える。
                let selected = highlight == Some(index);
                if selected {
                    let band = RECT {
                        left: 1,
                        right: width - 1,
                        ..rect
                    };
                    let brush = CreateSolidBrush(colorref(palette.selected_background));
                    FillRect(hdc, &band, brush);
                    let _ = DeleteObject(brush.into());
                }
                let (ink, ground) = if selected {
                    (palette.selected_text, palette.selected_background)
                } else {
                    (palette.text, palette.background)
                };
                draw_line(hdc, line, rect, ink, ground);
            }
        }

        if let (Some(previous), Some(font)) = (previous, font) {
            SelectObject(hdc, previous);
            let _ = DeleteObject(font.into());
        }
    }
}

/// 一行を描く。本文のあとに、注釈を薄い色で添える。
///
/// 注釈は [`NOTE_WIDTH`] で切り、はみ出す分は「…」にする。**一覧の窓が
/// 長い注釈で画面いっぱいに広がっては、候補が読めない。**
///
/// # Safety
///
/// `hdc` に書体が選ばれていること。
unsafe fn draw_line(hdc: HDC, line: &Line, rect: RECT, ink: u32, ground: u32) {
    // SAFETY: 呼び出し側の約束による。
    unsafe {
        SetTextColor(hdc, colorref(ink));
        let mut text: Vec<u16> = line.text.encode_utf16().collect();
        let mut area = rect;
        DrawTextW(
            hdc,
            &mut text,
            &mut area,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );

        let Some(note) = &line.note else {
            return;
        };
        let left = rect.left + text_width(hdc, &line.text) + scaled(NOTE_GAP);
        let mut area = RECT {
            left,
            right: (left + scaled(NOTE_WIDTH)).min(rect.right),
            ..rect
        };
        SetTextColor(hdc, colorref(mix(ink, ground)));
        let mut text: Vec<u16> = note.encode_utf16().collect();
        DrawTextW(
            hdc,
            &mut text,
            &mut area,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
}

/// 文字列の幅。
///
/// # Safety
///
/// `hdc` に書体が選ばれていること。
unsafe fn text_width(hdc: HDC, text: &str) -> i32 {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    // SAFETY: 呼び出し側の約束による。
    if unsafe { GetTextExtentPoint32W(hdc, &wide, &mut size) }.as_bool() {
        size.cx
    } else {
        0
    }
}

/// 二つの色の中間。注釈を本文より薄く見せるのに使う。
fn mix(a: u32, b: u32) -> u32 {
    let channel = |shift: u32| ((((a >> shift) & 0xFF) + ((b >> shift) & 0xFF)) / 2) << shift;
    channel(16) | channel(8) | channel(0)
}

/// 本文と注釈のあいだ。
const NOTE_GAP: i32 = 12;

/// 一覧の注釈を切る幅。これを超える分は「…」にする。
const NOTE_WIDTH: i32 = 240;

/// 注釈だけの窓を折り返す幅。
const WRAP_WIDTH: i32 = 360;

/// 窓に預ける、描くものと色の組。
struct Painted {
    content: Content,
    palette: Palette,
}

/// `0xRRGGBB` を `COLORREF` (`0x00BBGGRR`) にする。
fn colorref(rgb: u32) -> COLORREF {
    COLORREF(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF))
}

/// 窓の大きさを測る。
fn measure(content: &Content) -> (i32, i32) {
    // SAFETY: 画面の DC を借りて測り、すぐ返す。
    unsafe {
        let hdc = GetDC(None);
        let font = ui_font();
        let previous = font.map(|f| SelectObject(hdc, f.into()));

        let measured = if content.wraps() {
            // 折り返したときの大きさを、描く前に尋ねる。
            let text = content
                .lines()
                .into_iter()
                .next()
                .map(|line| line.text)
                .unwrap_or_default();
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: scaled(WRAP_WIDTH),
                bottom: 0,
            };
            DrawTextW(
                hdc,
                &mut wide,
                &mut rect,
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX | DT_CALCRECT,
            );
            (rect.right - rect.left, rect.bottom - rect.top)
        } else {
            let line_height = line_height(hdc);
            let lines = content.lines();
            let widest = lines
                .iter()
                .map(|line| {
                    let note = line.note.as_deref().map_or(0, |note| {
                        scaled(NOTE_GAP) + text_width(hdc, note).min(scaled(NOTE_WIDTH))
                    });
                    text_width(hdc, &line.text) + note
                })
                .max()
                .unwrap_or(0);
            let rows = i32::try_from(lines.len()).unwrap_or(1);
            (widest, line_height * rows)
        };

        if let (Some(previous), Some(font)) = (previous, font) {
            SelectObject(hdc, previous);
            let _ = DeleteObject(font.into());
        }
        ReleaseDC(None, hdc);

        (measured.0 + PADDING * 2, measured.1 + PADDING * 2)
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

/// 拡大率に合わせて伸ばす。
pub(crate) fn scaled(value: i32) -> i32 {
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
            notes: Vec::new(),
            number,
            count,
        }
    }

    #[test]
    fn a_note_sits_beside_its_candidate() {
        let mut page = page(&[('a', "橋"), ('s', "箸")], 1, 1);
        page.notes = vec![Some("bridge".to_owned()), None];
        let lines = page.lines();
        assert_eq!(lines[0].text, "a: 橋");
        assert_eq!(lines[0].note.as_deref(), Some("bridge"));
        assert_eq!(lines[1], "s: 箸", "注釈の無い候補はそのまま");
    }

    #[test]
    fn an_annotation_alone_is_wrapped_not_listed() {
        // 一つずつ見せているあいだの注釈は、長くても全文を折り返して出す。
        let content = Content::Annotation("とても長い注釈".repeat(20));
        assert!(content.wraps());
        assert!(!content.is_empty());
        assert!(!Content::Page(page(&[('a', "橋")], 1, 1)).wraps());
    }

    #[test]
    fn the_note_is_drawn_between_the_ink_and_the_ground() {
        assert_eq!(mix(0xFF_FF_FF, 0x00_00_00), 0x7F_7F_7F);
        assert_eq!(mix(0x20_40_60, 0x20_40_60), 0x20_40_60);
    }

    #[test]
    fn the_guess_shows_the_key_that_takes_it() {
        let line = Content::Completion(Completion {
            entries: vec!["漢字".to_owned()],
            take_key: '.',
            ..Completion::default()
        })
        .lines();
        assert_eq!(line, [".: 漢字"], "出すのは変換先");
    }

    #[test]
    fn walking_with_tab_lists_the_neighbours() {
        // 次に何が来るかが見えないと、何度押せばよいか分からない。
        let content = Content::Completion(Completion {
            taken: true,
            take_key: '.',
            entries: vec!["漢字".to_owned(), "患者".to_owned()],
            current: 1,
            number: 1,
            count: 1,
        });
        assert_eq!(content.lines(), ["漢字", "患者"]);
        // 印を足さず、行そのものを反転させる。
        assert_eq!(content.highlight(), Some(1));
    }

    #[test]
    fn a_taken_guess_shows_no_key() {
        // 同じキーが同じことをしないのに、案内を出したままにはできない。
        let lines = Content::Completion(Completion {
            taken: true,
            take_key: '.',
            entries: vec!["患者".to_owned()],
            current: 0,
            number: 1,
            count: 1,
        })
        .lines();
        assert!(lines.iter().all(|line| !line.text.contains(':')));
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
        assert_eq!(
            many.lines().last().map(|line| line.text.clone()).as_deref(),
            Some("2 / 3")
        );
    }

    #[test]
    fn an_empty_page_has_nothing_to_draw() {
        assert!(Content::Page(page(&[], 1, 1)).is_empty());
    }

    #[test]
    fn registration_shows_the_key_and_what_has_been_typed() {
        let line = Content::Registration(Registration {
            key: "かんじ".to_owned(),
            text: "漢字".to_owned(),
            depth: 1,
        })
        .lines();
        assert_eq!(line, vec!["[登録] かんじ: 漢字│"]);
    }

    #[test]
    fn nesting_is_shown_by_the_brackets() {
        let line = Content::Registration(Registration {
            key: "かんじ".to_owned(),
            text: String::new(),
            depth: 2,
        })
        .lines();
        assert!(
            line[0].text.starts_with("[[登録]]"),
            "登録の中の登録が一目で分かる: {}",
            line[0].text
        );
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
