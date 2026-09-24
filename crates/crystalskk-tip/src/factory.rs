//! COM のクラスファクトリ。
//!
//! `DllGetClassObject` が返すもので、TSF はこれを通して
//! [`crate::service::TextService`] を作る。

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::core::{BOOL, IUnknown, Interface, Ref, Result, implement};

use crate::service::TextService;

/// 生きているオブジェクトの数。
///
/// これが 0 でない間、DLL を取り外してはならない。
static OBJECT_COUNT: AtomicIsize = AtomicIsize::new(0);

/// DLL を取り外してよいか。
pub fn can_unload() -> bool {
    OBJECT_COUNT.load(Ordering::Acquire) <= 0
}

/// 生存数を一つ増やし、落ちるときに減らす見張り。
///
/// **この DLL が作る COM オブジェクトは、例外なくこれを欄に持つ。** 持たない
/// ものが一つでもあると、TSF やアプリがまだ握っているのに `DllCanUnloadNow`
/// が「降ろしてよい」と答えてしまう。アプリが `CoFreeUnusedLibraries` を
/// 呼んだ時点で DLL は降ろされ、次の呼び出しや、この DLL の中を指す窓の
/// 手続きへのメッセージが、消えた番地へ飛んで**アプリごと落ちる**。
/// 実際に X-Mouse Button Control がこれで落ちた。CorvusSKK も同じ位置で
/// `DllAddRef` / `DllRelease` を呼んでいる。
///
/// 中身を隠してあるので、[`ObjectGuard::new`] を通さずには作れない。
#[derive(Debug)]
pub struct ObjectGuard(());

impl ObjectGuard {
    pub fn new() -> Self {
        OBJECT_COUNT.fetch_add(1, Ordering::Release);
        Self(())
    }
}

impl Default for ObjectGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ObjectGuard {
    fn drop(&mut self) {
        OBJECT_COUNT.fetch_sub(1, Ordering::Release);
    }
}

/// [`TextService`] を作るファクトリ。
#[implement(IClassFactory)]
#[derive(Default)]
pub struct ClassFactory {
    /// 生きている間、DLL を降ろさせない。
    _alive: ObjectGuard,
}

impl std::fmt::Debug for ClassFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassFactory").finish()
    }
}

impl IClassFactory_Impl for ClassFactory_Impl {
    // 署名は COM が決めており、生ポインタを受け取る安全な関数にせざるを得ない。
    // 有効性は呼び出し側が保証する取り決めになっている。
    #[allow(
        clippy::not_unsafe_ptr_arg_deref,
        reason = "COM の呼び出し規約が引数の有効性を保証する"
    )]
    fn CreateInstance(
        &self,
        punkouter: Ref<IUnknown>,
        riid: *const windows::core::GUID,
        ppvobject: *mut *mut c_void,
    ) -> Result<()> {
        crate::guard::guard("CreateInstance", || {
            if ppvobject.is_null() {
                return Err(E_POINTER.into());
            }
            // SAFETY: null でないことを確かめた出力先を、まず空にする。
            unsafe { *ppvobject = std::ptr::null_mut() };

            // 集約 (aggregation) は使わない。
            if punkouter.is_some() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }

            let service: IUnknown = TextService::new().into();
            // SAFETY: `riid` と `ppvobject` は COM の約束どおり有効な場所を指す。
            unsafe { service.query(riid, ppvobject).ok() }
        })
    }

    /// COM は DLL を保持したいときにこれを呼ぶ。生存数に足し引きする。
    fn LockServer(&self, flock: BOOL) -> Result<()> {
        if flock.as_bool() {
            OBJECT_COUNT.fetch_add(1, Ordering::Release);
        } else {
            OBJECT_COUNT.fetch_sub(1, Ordering::Release);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guard_tracks_live_objects() {
        let before = OBJECT_COUNT.load(Ordering::Acquire);
        {
            let _guard = ObjectGuard::new();
            assert_eq!(OBJECT_COUNT.load(Ordering::Acquire), before + 1);
        }
        assert_eq!(OBJECT_COUNT.load(Ordering::Acquire), before);
    }
}
