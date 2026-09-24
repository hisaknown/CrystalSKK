//! COM の境界で失敗を受け止める。
//!
//! Rust のパニックが `extern "system"` の境界を越えると、その先は未定義
//! であり、実際には**プロセスごと落ちる**。TIP は他人のプロセスの中で
//! 動くので、これは入力先アプリを道連れにすることを意味する。
//!
//! PRD §3「ホストアプリを巻き込まない」を守れるかどうかは、ここで
//! 受け止められるかにかかっている。COM から呼ばれる関数の中身は、
//! 例外なくこれを通す。

use windows::Win32::Foundation::E_FAIL;
use windows::core::Result;

use crate::log;

/// COM から呼ばれた処理を包む。
///
/// パニックは記録して `E_FAIL` に変える。呼んできた相手には「失敗した」
/// としか伝わらないが、少なくとも巻き添えにはしない。
pub fn guard<T>(what: &str, body: impl FnOnce() -> Result<T>) -> Result<T> {
    // `AssertUnwindSafe` を使うのは、ここで包む対象がすべて COM の
    // 呼び出しであり、途中で壊れた状態を次の呼び出しへ持ち越しても
    // 「入力がおかしくなる」以上のことは起きないため。プロセスを
    // 落とすよりはるかにましである。
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
            log::error(&format!("{what} が失敗した: {}", error.message()));
            Err(error)
        }
        Err(payload) => {
            log::error(&format!("{what} でパニックした: {}", describe(&payload)));
            Err(E_FAIL.into())
        }
    }
}

/// パニックの中身を、読める形にする。
fn describe(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "理由は分からない".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_successful_body_passes_through() {
        let value = guard("試験", || Ok(42)).expect("成功する");
        assert_eq!(value, 42);
    }

    #[test]
    fn an_error_passes_through() {
        let error = guard::<()>("試験", || Err(E_FAIL.into())).expect_err("失敗する");
        assert_eq!(error.code(), E_FAIL);
    }

    #[test]
    fn a_panic_becomes_an_error_instead_of_taking_the_process_down() {
        let error = guard::<()>("試験", || panic!("わざと")).expect_err("失敗になる");
        assert_eq!(error.code(), E_FAIL);
    }
}
