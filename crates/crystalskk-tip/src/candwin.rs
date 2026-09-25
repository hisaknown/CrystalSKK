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
//! # 描き方
//!
//! 角を丸めた窓に、押すキーを枠で囲んで並べる。色は設定に従い、明るい組と暗い組をアプリの明るさで
//! 選ぶ (ADR-0026)。大きさは窓を出すモニターの拡大率に従う ([`crate::dpi`])。
//! 候補の注釈は本文の右に薄く添える (ADR-0027)。描き方は `paint` の一箇所に
//! 閉じてある。

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DeleteObject, DrawTextW, EndPaint,
    FillRect, FrameRect, GetDC, GetStockObject, GetTextExtentPoint32W, HDC, InvalidateRect,
    NULL_BRUSH, NULL_PEN, PAINTSTRUCT, PS_SOLID, ReleaseDC, RoundRect, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow,
    GWLP_HWNDPARENT, GWLP_USERDATA, GetSystemMetrics, GetWindowLongPtrW, HWND_TOPMOST,
    IsWindowVisible, RegisterClassExW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, UnregisterClassW, WINDOW_EX_STYLE, WM_DESTROY, WM_PAINT, WNDCLASSEXW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::dpi;
use crate::guard::guard;
use crate::log;
use crate::popup;
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

    /// 本文の左に置くもの。
    fn lead(&self) -> Lead {
        match self {
            Self::Page(_) => Lead::Key,
            Self::Completion(completion) if completion.taken => Lead::Mark,
            Self::Completion(_) => Lead::Key,
            _ => Lead::None,
        }
    }

    /// 帯を敷いて見せる行。無ければ `None`。
    ///
    /// **記号で示すより、地の色を変えるほうがよい。** 記号は
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

/// 窓の一行。押すキーと本文と、その右に薄く添える注釈。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// この行を選ぶキー。本文の左に、枠で囲んで出す。
    pub key: Option<char>,
    pub text: String,
    pub note: Option<String>,
}

impl Line {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            key: None,
            text: text.into(),
            note: None,
        }
    }

    fn keyed(key: char, text: impl Into<String>) -> Self {
        Self {
            key: Some(key),
            ..Self::plain(text)
        }
    }
}

impl PartialEq<&str> for Line {
    fn eq(&self, other: &&str) -> bool {
        self.key.is_none() && self.note.is_none() && self.text == *other
    }
}

/// 本文の左に何を置くか。**一つの窓の中では、どの行も同じ幅を空ける。**
/// 行によって本文の書き出しがずれると、目が候補を追えない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lead {
    /// 何も置かない。
    None,
    /// 押すキーを枠で囲んで置く。
    Key,
    /// 選んでいる行にだけ細い印を置く。選んでいない行も同じ幅を空ける。
    Mark,
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
            // 一覧と同じく、押すキーを左に出す。
            return self
                .entries
                .first()
                .map(|word| Line::keyed(self.take_key, word.clone()))
                .into_iter()
                .collect();
        }

        // 受け取った後は打鍵の案内を出さない。**同じキーが同じことを
        // しないのに、出したままにはできない。** 選んでいる候補は
        // [`Content::highlight`] が帯を敷く。
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
                key: Some(*label),
                text: text.clone(),
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
    /// 角を DWM が丸めているか。Windows 10 では丸められない。
    rounded: Cell<bool>,
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
            rounded: Cell::new(false),
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
        // 窓を出すモニターの拡大率で描く (`crate::dpi`)。
        let dpi = dpi::at(POINT {
            x: anchor.left,
            y: anchor.bottom,
        });
        let rounded = self.rounded.get();
        if rounded {
            popup::set_border(hwnd, palette.border);
        }
        let stored = Box::into_raw(Box::new(Painted {
            content: content.clone(),
            palette,
            dpi,
            rounded,
        }));
        // SAFETY: 直前に作った箱を預け、前に預けていた分はここで落とす。
        unsafe {
            let previous = SetWindowLongPtrW(hwnd, GWLP_USERDATA, stored as isize);
            if previous != 0 {
                drop(Box::from_raw(previous as *mut Painted));
            }
        }

        let (width, height) = measure(content, dpi);
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

    /// 窓が出ているか。
    pub fn is_visible(&self) -> bool {
        let hwnd = *self.hwnd.borrow();
        // SAFETY: 尋ねるだけ。
        !hwnd.is_invalid() && unsafe { IsWindowVisible(hwnd) }.as_bool()
    }

    /// 中身はそのままに、`anchor` のそばへ動かす。出ていなければ何もしない。
    ///
    /// 動いた先のモニターの拡大率が違えば、その大きさで描き直す。
    pub fn follow(&self, anchor: RECT) {
        if !self.is_visible() {
            return;
        }
        let hwnd = *self.hwnd.borrow();
        // SAFETY: 預けてあるのは `show` で作った箱で、窓があるあいだ生きている。
        let painted =
            unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Painted).as_ref() }
                .map(|painted| (painted.content.clone(), painted.palette));
        if let Some((content, palette)) = painted {
            self.show(&content, anchor, None, palette);
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
                self.rounded.set(popup::round_corners(hwnd));
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
            //
            // 影も付ける。アプリの上に浮いていることが一目で分かる。
            style: CS_HREDRAW | CS_VREDRAW | CS_DROPSHADOW,
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
                        paint(
                            hdc,
                            &painted.content,
                            painted.palette,
                            painted.dpi,
                            painted.rounded,
                        );
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
/// `rounded` は、角を DWM が丸めているか。丸めているなら枠も DWM が描く。
///
/// # Safety
///
/// `hdc` が描画中のものであること。
unsafe fn paint(hdc: HDC, content: &Content, palette: Palette, dpi: u32, rounded: bool) {
    // SAFETY: 呼び出し側の約束による。作ったものはこの関数の中で片付ける。
    unsafe {
        let font = dpi::message_font(dpi);
        let previous = font.map(|f| SelectObject(hdc, f.into()));

        let (width, height) = measure(content, dpi);
        let area = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };

        let background = CreateSolidBrush(colorref(palette.background));
        FillRect(hdc, &area, background);
        let _ = DeleteObject(background.into());

        // 枠。地と同じ色では、背景に溶けて境目が分からない。角を丸めて
        // いるなら DWM が丸みに沿って描くので、四角い枠は重ねない。
        if !rounded {
            let border = CreateSolidBrush(colorref(palette.border));
            FrameRect(hdc, &area, border);
            let _ = DeleteObject(border.into());
        }

        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, colorref(palette.text));

        let inset = dpi::scale(WINDOW_PAD, dpi);
        let row_pad = dpi::scale(ROW_PAD, dpi);

        // 注釈だけの窓は、窓の幅で折り返して全文を出す。
        if content.wraps() {
            let pad = inset + row_pad;
            let text = content
                .lines()
                .into_iter()
                .next()
                .map(|line| line.text)
                .unwrap_or_default();
            let mut rect = RECT {
                left: pad,
                top: pad,
                right: width - pad,
                bottom: height - pad,
            };
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            DrawTextW(
                hdc,
                &mut wide,
                &mut rect,
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
            );
        } else {
            let metrics = Metrics::of(hdc, dpi);
            let lead = content.lead();
            let highlight = content.highlight();
            for (index, line) in content.lines().iter().enumerate() {
                let top = inset + metrics.line * i32::try_from(index).unwrap_or(0);
                let row = RECT {
                    left: inset,
                    top,
                    right: width - inset,
                    bottom: top + metrics.line,
                };

                // 選んでいる行には帯を敷き、左端に印を立てる。
                //
                // 帯は窓の幅いっぱいに敷く。文字の幅だけ塗ると、行によって
                // 帯の長さが変わってちらついて見える。
                let selected = highlight == Some(index);
                if selected {
                    round_fill(
                        hdc,
                        row,
                        palette.selected_background,
                        dpi::scale(BAND_ROUND, dpi),
                    );
                    let mark_height = metrics.line / 2;
                    let mark_top = row.top + (metrics.line - mark_height) / 2;
                    let mark_left = row.left + dpi::scale(MARK_OFFSET, dpi);
                    let mark_width = dpi::scale(MARK_WIDTH, dpi);
                    let mark = RECT {
                        left: mark_left,
                        top: mark_top,
                        right: mark_left + mark_width,
                        bottom: mark_top + mark_height,
                    };
                    round_fill(hdc, mark, palette.key, mark_width);
                }
                let (ink, ground) = if selected {
                    (palette.selected_text, palette.selected_background)
                } else {
                    (palette.text, palette.background)
                };

                let left = row.left + row_pad;
                if let (Lead::Key, Some(key)) = (lead, line.key) {
                    let key_box = KeyBox {
                        left,
                        row,
                        side: metrics.key,
                    };
                    draw_key(hdc, key, key_box, palette.key, mix(ink, ground));
                }
                let text = RECT {
                    left: left + metrics.lead(lead, dpi),
                    right: row.right - row_pad,
                    ..row
                };
                draw_line(hdc, line, text, ink, ground, dpi);
            }
        }

        if let (Some(previous), Some(font)) = (previous, font) {
            SelectObject(hdc, previous);
            let _ = DeleteObject(font.into());
        }
    }
}

/// 角の丸い四角を塗る。`round` は角の丸みの直径。
///
/// # Safety
///
/// `hdc` が描画中のものであること。
unsafe fn round_fill(hdc: HDC, rect: RECT, color: u32, round: i32) {
    // SAFETY: 呼び出し側の約束による。選んだものは戻し、作ったものは消す。
    unsafe {
        let brush = CreateSolidBrush(colorref(color));
        let previous_brush = SelectObject(hdc, brush.into());
        let previous_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
        // 線を引かないと、右と下が一画素ずつ欠ける。その分を足す。
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right + 1,
            rect.bottom + 1,
            round,
            round,
        );
        SelectObject(hdc, previous_pen);
        SelectObject(hdc, previous_brush);
        let _ = DeleteObject(brush.into());
    }
}

/// キーを囲む枠の置き場所。`left` から始まる一辺 `side` の正方形を、
/// `row` の縦の真ん中に置く。
#[derive(Debug, Clone, Copy)]
struct KeyBox {
    left: i32,
    row: RECT,
    side: i32,
}

/// 押すキーを、角の丸い枠で囲んで描く。
///
/// **枠の大きさはキーによらず同じにする。** キーごとに幅が変わると、本文の
/// 書き出しがずれる。
///
/// # Safety
///
/// `hdc` に書体が選ばれていること。
unsafe fn draw_key(hdc: HDC, key: char, place: KeyBox, frame: u32, ink: u32) {
    // SAFETY: 呼び出し側の約束による。選んだものは戻し、作ったものは消す。
    unsafe {
        let top = place.row.top + (place.row.bottom - place.row.top - place.side) / 2;
        let mut rect = RECT {
            left: place.left,
            top,
            right: place.left + place.side,
            bottom: top + place.side,
        };
        let pen = CreatePen(PS_SOLID, 1, colorref(frame));
        let previous_pen = SelectObject(hdc, pen.into());
        let previous_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
        let round = place.side / 3;
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            round,
            round,
        );
        SelectObject(hdc, previous_brush);
        SelectObject(hdc, previous_pen);
        let _ = DeleteObject(pen.into());

        // キーボードの刻印に合わせて大文字で出す。
        SetTextColor(hdc, colorref(ink));
        let mut text: Vec<u16> = key
            .to_ascii_uppercase()
            .to_string()
            .encode_utf16()
            .collect();
        DrawTextW(
            hdc,
            &mut text,
            &mut rect,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
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
unsafe fn draw_line(hdc: HDC, line: &Line, rect: RECT, ink: u32, ground: u32, dpi: u32) {
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
        let left = rect.left + text_width(hdc, &line.text) + dpi::scale(NOTE_GAP, dpi);
        let mut area = RECT {
            left,
            right: (left + dpi::scale(NOTE_WIDTH, dpi)).min(rect.right),
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
    /// 描く拡大率。出すときに、出すモニターで決める。
    dpi: u32,
    /// 角を DWM が丸めているか。
    rounded: bool,
}

/// `0xRRGGBB` を `COLORREF` (`0x00BBGGRR`) にする。
fn colorref(rgb: u32) -> COLORREF {
    COLORREF(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF))
}

/// 窓の大きさを測る。
fn measure(content: &Content, dpi: u32) -> (i32, i32) {
    // SAFETY: 画面の DC を借りて測り、すぐ返す。
    unsafe {
        let hdc = GetDC(None);
        let font = dpi::message_font(dpi);
        let previous = font.map(|f| SelectObject(hdc, f.into()));
        let row_pad = dpi::scale(ROW_PAD, dpi);

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
                right: dpi::scale(WRAP_WIDTH, dpi),
                bottom: 0,
            };
            DrawTextW(
                hdc,
                &mut wide,
                &mut rect,
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX | DT_CALCRECT,
            );
            (
                rect.right - rect.left + row_pad * 2,
                rect.bottom - rect.top + row_pad * 2,
            )
        } else {
            let metrics = Metrics::of(hdc, dpi);
            let lead = metrics.lead(content.lead(), dpi);
            let lines = content.lines();
            let widest = lines
                .iter()
                .map(|line| {
                    let note = line.note.as_deref().map_or(0, |note| {
                        dpi::scale(NOTE_GAP, dpi)
                            + text_width(hdc, note).min(dpi::scale(NOTE_WIDTH, dpi))
                    });
                    lead + text_width(hdc, &line.text) + note
                })
                .max()
                .unwrap_or(0);
            let rows = i32::try_from(lines.len()).unwrap_or(1);
            (widest + row_pad * 2, metrics.line * rows)
        };

        if let (Some(previous), Some(font)) = (previous, font) {
            SelectObject(hdc, previous);
            let _ = DeleteObject(font.into());
        }
        ReleaseDC(None, hdc);

        // 行は窓の縁から少し離して並べる。帯の丸みが縁に食われないように。
        let inset = dpi::scale(WINDOW_PAD, dpi);
        (measured.0 + inset * 2, measured.1 + inset * 2)
    }
}

/// 行の寸法。どれも書体の高さから決める。
#[derive(Debug, Clone, Copy)]
struct Metrics {
    /// 一行の高さ。
    line: i32,
    /// キーを囲む枠の一辺。
    key: i32,
}

impl Metrics {
    /// # Safety
    ///
    /// `hdc` に測りたい書体が選ばれていること。
    unsafe fn of(hdc: HDC, dpi: u32) -> Self {
        let sample: Vec<u16> = "あA".encode_utf16().collect();
        let mut size = SIZE::default();
        // SAFETY: 呼び出し側の約束による。
        let height = if unsafe { GetTextExtentPoint32W(hdc, &sample, &mut size) }.as_bool() {
            size.cy
        } else {
            dpi::scale(FALLBACK_TEXT_HEIGHT, dpi)
        };
        Self {
            line: height + dpi::scale(LINE_GAP, dpi),
            key: height + dpi::scale(KEY_GROW, dpi),
        }
    }

    /// 本文の左に空ける幅。
    fn lead(self, lead: Lead, dpi: u32) -> i32 {
        match lead {
            Lead::None => 0,
            Lead::Key => self.key + dpi::scale(KEY_GAP, dpi),
            Lead::Mark => dpi::scale(MARK_SPACE, dpi),
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

/// 窓の縁と帯の間。
const WINDOW_PAD: i32 = 4;

/// 帯の縁と文字の間。
const ROW_PAD: i32 = 8;

/// 一行の高さのうち、文字の上下に空ける分。
const LINE_GAP: i32 = 12;

/// 帯の角の丸みの直径。
const BAND_ROUND: i32 = 8;

/// キーを囲む枠が、文字の高さより大きい分。
const KEY_GROW: i32 = 4;

/// キーを囲む枠と本文の間。
const KEY_GAP: i32 = 10;

/// 選んでいる行の印の太さ。
const MARK_WIDTH: i32 = 3;

/// 帯の縁から印までの距離。
const MARK_OFFSET: i32 = 3;

/// 印のために本文の左に空ける幅。選んでいない行も同じだけ空ける。
const MARK_SPACE: i32 = 6;

/// 書体を測れなかったときの文字の高さ。
const FALLBACK_TEXT_HEIGHT: i32 = 16;

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
        assert_eq!(lines[0].text, "橋");
        assert_eq!(lines[0].note.as_deref(), Some("bridge"));
        assert_eq!(lines[1], Line::keyed('s', "箸"), "注釈の無い候補はそのまま");
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
        assert_eq!(line, [Line::keyed('.', "漢字")], "出すのは変換先");
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
        // 選んでいる行に帯を敷く。キーは出さず、印の幅だけ空ける。
        assert_eq!(content.highlight(), Some(1));
        assert_eq!(content.lead(), Lead::Mark);
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
        assert!(lines.iter().all(|line| line.key.is_none()));
    }

    #[test]
    fn each_entry_becomes_a_line() {
        let page = page(&[('a', "漢字"), ('s', "感じ")], 1, 1);
        assert_eq!(
            page.lines(),
            vec![Line::keyed('a', "漢字"), Line::keyed('s', "感じ")]
        );
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
