//! COM のアパートメント。
//!
//! 入力方式の登録は COM 越しに行うため、呼び出す前にスレッドを
//! アパートメントに入れておく必要がある。出るのを忘れないよう、
//! 持ち手が落ちるときに自動で出る形にしてある。
//!
//! これを置いてあるおかげで、登録を呼ぶ側は `unsafe` を書かずに済む。

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

/// 単一スレッドアパートメントへの出入りを持つ。
///
/// すでに別の方式で初期化されていた場合は、そのまま使い、後始末もしない。
#[derive(Debug)]
pub struct Apartment {
    /// この持ち手が初期化したなら真。後始末するかどうかの判断に使う。
    owned: bool,
}

impl Apartment {
    /// アパートメントに入る。
    pub fn enter() -> Self {
        // SAFETY: 後始末は `Drop` で対にしてある。
        let owned = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        Self { owned }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: 自分が初期化したときだけ出る。
            unsafe { CoUninitialize() };
        }
    }
}
