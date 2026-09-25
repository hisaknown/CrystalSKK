//! CrystalSKK の TSF テキスト入力プロセッサ (TIP)。
//!
//! このクレートは COM のインプロセスサーバーであり、**入力先アプリの
//! プロセスに読み込まれる**。Word にも Chrome にもこのコードが同居する。
//! したがってここは薄く保ち、失敗しうる処理や重い処理を持ち込まない
//! (PRD §3「ホストアプリを巻き込まない」)。
//!
//! いまは登録して言語バーに現れるところまでで、入力はまだ行わない。
//!
//! # 登録
//!
//! ```text
//! regsvr32 crystalskk_tip.dll
//! regsvr32 /u crystalskk_tip.dll
//! ```
//!
//! 登録先は `HKEY_CURRENT_USER` なので管理者権限は要らない (ADR-0006)。

#![cfg(windows)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, E_POINTER, HMODULE, S_FALSE, S_OK};
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::System::SystemServices::{DLL_PROCESS_ATTACH, DLL_PROCESS_DETACH};
use windows::core::{BOOL, GUID, HRESULT, Interface};

pub mod candwin;
pub mod com;
pub mod compartment;
pub mod dialog;
pub mod dict;
pub mod display;
pub mod dpi;
pub mod edit;
pub mod factory;
pub mod guard;
pub mod guids;
pub mod icon;
pub mod indicator;
pub mod keys;
pub mod langbar;
pub mod launch;
pub mod log;
pub mod menu;
pub mod popup;
pub mod preserved;
pub mod profile;
pub mod registry;
pub mod service;
pub mod theme;
pub mod uielement;

use factory::ClassFactory;
use guids::CLSID_CRYSTALSKK;

/// 読み込まれたこの DLL のハンドル。`DllMain` で受け取る。
static MODULE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn module() -> HMODULE {
    HMODULE(MODULE.load(Ordering::Acquire) as *mut c_void)
}

/// DLL の出入り口。
///
/// ここでできることは強く制限されている。COM の呼び出しもロックの取得も
/// 行ってはならない。入るときはハンドルを覚えるだけにする。
///
/// # 降りるときに窓の種別を外す
///
/// 候補の窓の種別 ([`candwin`]) は、窓の手続きとして**この DLL の中の
/// 関数を指している**。種別を残したまま DLL が降ろされると、その指し先が
/// 消える。次に誰かが同じ名前の窓を作れば、無い関数へ飛ぶことになる。
///
/// 種別の登録は使うときまで遅らせているが、**外すのはここでしかできない**。
/// CorvusSKK も同じ場所で外している。
#[unsafe(no_mangle)]
pub extern "system" fn DllMain(module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(module.0 as usize, Ordering::Release);
    }
    if reason == DLL_PROCESS_DETACH {
        candwin::unregister_class();
        theme::unregister_class();
        indicator::unregister_class();
    }
    true.into()
}

/// COM がクラスオブジェクトを要求する入口。
///
/// # Safety
///
/// COM の規約どおり、`rclsid` と `riid` は有効な GUID を、`ppv` は
/// ポインタを書き込める場所を指していなければならない。
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || rclsid.is_null() || riid.is_null() {
        return E_POINTER;
    }
    // SAFETY: null でないことを確かめた出力先を空にしておく。
    unsafe { *ppv = std::ptr::null_mut() };

    // SAFETY: 呼び出し側が有効な GUID を指していることは COM の約束。
    if unsafe { *rclsid } != CLSID_CRYSTALSKK {
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    let factory: IClassFactory = ClassFactory::default().into();
    // SAFETY: `riid` と `ppv` は上で確かめた有効な場所を指す。
    unsafe { factory.query(riid, ppv) }
}

/// DLL を取り外してよいか COM が尋ねる。
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if factory::can_unload() { S_OK } else { S_FALSE }
}

/// 自己登録。`regsvr32` から呼ばれる。
#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    with_com(|| {
        let path = registry::module_path(module())?;
        registry::register_class(&path)?;
        profile::register_profile(&path)
    })
}

/// 登録の取り消し。`regsvr32 /u` から呼ばれる。
#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    with_com(|| {
        // 入力方式を先に消す。クラス登録が残っていないと消せないため。
        let profile = profile::unregister_profile();
        let class = registry::unregister_class();
        profile.and(class)
    })
}

/// COM を用意してから処理を行う。
///
/// `regsvr32` が COM を初期化しているとは限らないので自分で行う。
fn with_com(body: impl FnOnce() -> windows::core::Result<()>) -> HRESULT {
    let _apartment = com::Apartment::enter();
    match body() {
        Ok(()) => S_OK,
        Err(e) => e.code(),
    }
}
