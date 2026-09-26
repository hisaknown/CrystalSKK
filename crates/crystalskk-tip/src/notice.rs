//! 全体に配られる知らせを聞く窓。
//!
//! 二つの知らせを聞く。
//!
//! - タスクバーの明るさが変わった (`WM_SETTINGCHANGE`)。入力モードの絵を
//!   描き直させる ([`crate::theme`])。
//! - 設定ファイルが変わった。辞書サーバが全ウィンドウへ送る独自の
//!   メッセージで、受けたら設定の写しを取り直す (ADR-0040)。
//!
//! 全体に配られる知らせは、**画面に出ていないふつうの窓**には届くが、
//! 知らせ専用の窓 (`HWND_MESSAGE`) には届かない。そこで、見えない窓を
//! 一枚持つ。

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetWindowLongPtrW, MSGFLT_ALLOW, RegisterClassExW, RegisterWindowMessageW, SetWindowLongPtrW,
    UnregisterClassW, WINDOW_EX_STYLE, WM_DESTROY, WM_SETTINGCHANGE, WNDCLASSEXW, WS_EX_TOOLWINDOW,
    WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::guard::guard;
use crate::log;

/// 知らせを受けたときの手続き。
pub struct Handlers {
    /// タスクバーの明るさが変わった。
    pub theme: Box<dyn Fn()>,
    /// 設定ファイルが変わった。
    pub settings: Box<dyn Fn()>,
}

impl std::fmt::Debug for Handlers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handlers").finish_non_exhaustive()
    }
}

/// 知らせを聞く窓。落とすと窓も壊す。
pub struct Listener {
    hwnd: RefCell<HWND>,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener").finish_non_exhaustive()
    }
}

impl Listener {
    /// 窓を作って聞き始める。作れなければ `None`。
    ///
    /// 作れなくても入力には障らない。テーマを変えたときや設定を書き換えた
    /// とき、次に入力先を切り替えるまで古いままになるだけである。
    pub fn start(handlers: Handlers) -> Option<Self> {
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
                log::error(&format!("知らせを聞く窓を作れなかった: {}", e.message()));
                return None;
            }
        };
        // 管理者として動くアプリでは、ふつうの権限の辞書サーバからの知らせが
        // 窓に届かない (UIPI)。この一つだけは通す。
        if let Some(message) = settings_changed_message() {
            //
            // 隔離された入れ物 (app container) の中では、この操作そのものが
            // 拒まれる。ただしそこでは、権限の高いサーバから低い側への送信なので、
            // 許さなくても届く。**誤りとしては記録しない。**
            //
            // SAFETY: 自分で作った窓に、受け取るメッセージを一つ足すだけ。
            if let Err(e) =
                unsafe { ChangeWindowMessageFilterEx(hwnd, message, MSGFLT_ALLOW, None) }
            {
                log::trace(&format!(
                    "設定の知らせを通す許可を足せなかった (隔離された入れ物なら要らない): {}",
                    e.message()
                ));
            }
        }
        let handlers: Box<Handlers> = Box::new(handlers);
        // SAFETY: 預けた箱は窓が壊れるときに引き取る。
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(handlers) as _);
        }
        Some(Self {
            hwnd: RefCell::new(hwnd),
        })
    }
}

impl Drop for Listener {
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

/// 設定ファイルが変わったという知らせの番号。
///
/// 名前から Windows に番号を振ってもらう。辞書サーバも同じ名前で番号を
/// 得るので、プロセスをまたいで同じ番号になる。
fn settings_changed_message() -> Option<u32> {
    use std::sync::OnceLock;
    static MESSAGE: OnceLock<u32> = OnceLock::new();

    let message = *MESSAGE.get_or_init(|| {
        let name = HSTRING::from(crystalskk_ipc::SETTINGS_CHANGED_MESSAGE);
        // SAFETY: 終端のある名前を渡している。
        unsafe { RegisterWindowMessageW(&name) }
    });
    (message != 0).then_some(message)
}

/// 窓の種別の名前。
const CLASS_NAME: PCWSTR = w!("CrystalSKKNoticeListener");

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
            log::error("知らせを聞く窓の種別を登録できなかった");
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

/// 預けた手続きを呼ぶ。
///
/// パニックを窓の手続きから外へ出すと、アプリごと落ちる。
fn call(hwnd: HWND, what: &str, pick: impl Fn(&Handlers) -> &dyn Fn()) {
    let _ = guard(what, || {
        // SAFETY: 預けた箱は窓が生きている間は有効。
        let handlers =
            unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Handlers).as_ref() };
        if let Some(handlers) = handlers {
            log::write(what);
            pick(handlers)();
        }
        Ok(())
    });
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
            call(hwnd, "タスクバーの明るさが変わった", |h| {
                &*h.theme
            });
            LRESULT(0)
        }
        _ if Some(message) == settings_changed_message() => {
            call(hwnd, "設定ファイルが変わったと知らされた", |h| &*h.settings);
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: 預けたのは自分の箱。二度落とさないよう 0 に戻す。
            unsafe {
                let stored = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if stored != 0 {
                    drop(Box::from_raw(stored as *mut Handlers));
                }
            }
            LRESULT(0)
        }
        // SAFETY: 既定の処理に委ねる。
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
