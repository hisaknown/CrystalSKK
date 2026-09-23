//! タスクバーの明るさに合わせる。
//!
//! 入力モードの絵は単色で描く。Windows 標準の IME と同じく、タスクバーが
//! 明るければ黒、暗ければ白にする。**IME がテーマ別の絵を渡す口は無い**
//! ので、こちらでテーマを読み、変わったら描き直させる。
//!
//! 見るのはアプリの明るさ (`AppsUseLightTheme`) ではなく、**タスクバーの
//! 明るさ** (`SystemUsesLightTheme`) である。絵が載るのはタスクバーで、
//! 二つは別々に選べる。

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_HIGHLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor, SYS_COLOR_INDEX,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
    RegisterClassExW, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowLongPtrW,
    SystemParametersInfoW, UnregisterClassW, WINDOW_EX_STYLE, WM_DESTROY, WM_SETTINGCHANGE,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::guard::guard;
use crate::log;

/// タスクバーの明るさ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    /// いまのタスクバーの明るさ。
    ///
    /// 読めなければ暗いほうとする。Windows 10 の 1903 からの既定がそれで、
    /// 値が無いのはそれより古いときである (そのころのタスクバーも暗い)。
    pub fn current() -> Self {
        read(w!("SystemUsesLightTheme"), Self::Dark)
    }

    /// いまのアプリの明るさ。カーソルのそばに出す窓は、こちらに合わせる。
    ///
    /// 窓が載るのはタスクバーではなくアプリの上である。二つは別々に選べる。
    /// 読めなければ明るいほうとする (アプリの既定)。
    pub fn apps() -> Self {
        read(w!("AppsUseLightTheme"), Self::Light)
    }

    /// 絵を描く色。`0xRRGGBB`。
    ///
    /// **明るいタスクバーには黒、暗いタスクバーには白。** 色を設定で
    /// 選べるようにするなら、ここが差し替わる。
    pub fn ink(self) -> u32 {
        match self {
            Self::Light => 0x00_00_00,
            Self::Dark => 0xFF_FF_FF,
        }
    }
}

/// `Personalize` の下の明暗の値を読む。1 なら明るい。
fn read(name: PCWSTR, fallback: Theme) -> Theme {
    let mut value: u32 = 0;
    let mut size = u32::try_from(size_of::<u32>()).unwrap_or(4);
    // SAFETY: 書き込み先はこの関数の変数で、大きさも渡している。
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            name,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&raw mut size),
        )
    };
    match (status.is_ok(), value) {
        (false, _) => fallback,
        (true, 0) => Theme::Dark,
        (true, _) => Theme::Light,
    }
}

/// アプリの上に出す小窓の色。どれも `0xRRGGBB`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub background: u32,
    pub text: u32,
    pub border: u32,
}

impl Palette {
    /// いまの明るさに合わせた色。
    ///
    /// - **ハイコントラストなら、システムの色に従う。** 利用者が選んだ配色を
    ///   上書きしてはならない。
    /// - 明るいなら、システムの色 (窓の地、文字、強調)。
    /// - 暗いなら、黒い地に白い文字。**Win32 のシステムの色はダーク
    ///   モードにしても白いまま**なので、こちらで決める。
    pub fn current() -> Self {
        if high_contrast() || Theme::apps() == Theme::Light {
            return Self::system();
        }
        Self {
            background: DARK_BACKGROUND,
            text: 0xFF_FF_FF,
            border: system(COLOR_HIGHLIGHT),
        }
    }

    fn system() -> Self {
        Self {
            background: system(COLOR_WINDOW),
            text: system(COLOR_WINDOWTEXT),
            border: system(COLOR_HIGHLIGHT),
        }
    }
}

/// 暗いときの地の色。Windows 11 の暗い小窓 (メニューやツールチップ) に近い。
const DARK_BACKGROUND: u32 = 0x2B_2B_2B;

/// システムの色を `0xRRGGBB` で。
fn system(index: SYS_COLOR_INDEX) -> u32 {
    // SAFETY: 番号を渡して色を受け取るだけ。
    let c = unsafe { GetSysColor(index) };
    ((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF)
}

/// ハイコントラストの配色が選ばれているか。
fn high_contrast() -> bool {
    let mut info = HIGHCONTRASTW {
        cbSize: u32::try_from(size_of::<HIGHCONTRASTW>()).unwrap_or(0),
        ..Default::default()
    };
    // SAFETY: 大きさを告げた構造体へ書かせる。
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            info.cbSize,
            Some(std::ptr::from_mut::<HIGHCONTRASTW>(&mut info).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    ok && info.dwFlags.contains(HCF_HIGHCONTRASTON)
}

/// 見出しの変化を聞き、変わったら知らせる。
///
/// 全体に配られる知らせ (`WM_SETTINGCHANGE`) は、**画面に出ていない
/// ふつうの窓**には届くが、知らせ専用の窓 (`HWND_MESSAGE`) には届かない。
/// そこで、見えない窓を一枚持つ。
pub struct Watcher {
    hwnd: RefCell<HWND>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").finish_non_exhaustive()
    }
}

/// 窓に預ける、知らせを受けたときの手続き。
type Handler = Box<dyn Fn()>;

impl Watcher {
    /// テーマが変わったら `changed` を呼ぶ。作れなければ `None`。
    ///
    /// 作れなくても入力には障らない。テーマを変えたとき、次に入力先を
    /// 切り替えるまで絵が古いままになるだけである。
    pub fn start(changed: impl Fn() + 'static) -> Option<Self> {
        register_class()?;
        // SAFETY: 種別は直前に登録したもの。見えない窓を作る。
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0),
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
        let hwnd = match hwnd {
            Ok(hwnd) => hwnd,
            Err(e) => {
                log::error(&format!("テーマを見張る窓を作れなかった: {}", e.message()));
                return None;
            }
        };
        let handler: Box<Handler> = Box::new(Box::new(changed));
        // SAFETY: 預けた箱は窓が壊れるときに引き取る。
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(handler) as isize);
        }
        Some(Self {
            hwnd: RefCell::new(hwnd),
        })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let hwnd = std::mem::take(&mut *self.hwnd.borrow_mut());
        if !hwnd.is_invalid() {
            // SAFETY: 自分で作った窓を壊す。預けた箱は WM_DESTROY で引き取る。
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

/// 窓の種別の名前。
const CLASS_NAME: PCWSTR = w!("CrystalSKKThemeWatcher");

fn register_class() -> Option<()> {
    use std::sync::OnceLock;
    static REGISTERED: OnceLock<bool> = OnceLock::new();

    let ok = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or(0),
            lpfnWndProc: Some(window_proc),
            lpszClassName: CLASS_NAME,
            hInstance: crate::module().into(),
            ..Default::default()
        };
        // SAFETY: 名前も手続きもこのモジュールのもの。
        let atom = unsafe { RegisterClassExW(&class) };
        if atom == 0 {
            log::error("テーマを見張る窓の種別を登録できなかった");
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

/// 明るさの切り替えを伝える知らせか。
///
/// 明暗が変わると、`WM_SETTINGCHANGE` が `"ImmersiveColorSet"` を添えて
/// 配られる。
///
/// # Safety
///
/// `lparam` は `WM_SETTINGCHANGE` に添えられたもの (null か、終端のある
/// 幅広文字列)。
unsafe fn is_color_change(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    // SAFETY: 呼び出し側の約束による。
    let text = unsafe { PCWSTR(lparam.0 as *const u16).to_string() };
    text.is_ok_and(|text| text == "ImmersiveColorSet")
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
        // SAFETY: WM_SETTINGCHANGE の lparam である。
        WM_SETTINGCHANGE if unsafe { is_color_change(lparam) } => {
            // パニックを窓の手続きから外へ出すと、アプリごと落ちる。
            let _ = guard("テーマの変化", || {
                // SAFETY: 預けた箱は窓が生きている間は有効。
                let handler =
                    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Handler).as_ref() };
                if let Some(handler) = handler {
                    log::write("タスクバーの明るさが変わった");
                    handler();
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
                    drop(Box::from_raw(stored as *mut Handler));
                }
            }
            LRESULT(0)
        }
        // SAFETY: 既定の処理に委ねる。
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ink_stands_out_from_the_taskbar() {
        assert_eq!(Theme::Light.ink(), 0x00_00_00, "明るいタスクバーには黒");
        assert_eq!(Theme::Dark.ink(), 0xFF_FF_FF, "暗いタスクバーには白");
    }

    #[test]
    fn the_theme_can_be_read() {
        // どちらになるかは機械次第。読めて、落ちないこと。
        let _ = Theme::current();
        let _ = Theme::apps();
        let _ = Palette::current();
    }

    #[test]
    fn a_dark_popup_is_light_on_dark() {
        let dark = Palette {
            background: DARK_BACKGROUND,
            text: 0xFF_FF_FF,
            border: 0,
        };
        // 地より文字のほうが十分明るい。
        assert!(dark.text > dark.background + 0x80_80_80);
    }
}
