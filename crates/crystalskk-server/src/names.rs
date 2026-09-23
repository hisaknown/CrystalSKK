//! 待ち合わせ場所の名前。
//!
//! パイプもミューテックスも**機械の中で名前を共有する**。同じ機械に別の
//! 利用者がログオンしていれば、それぞれのサーバが立つ。名前が同じだと、
//! 先に立ったほうしか居られず、**他人の辞書を引くことになりかねない**。
//!
//! そこで名前に利用者の SID を入れる。利用者ごとに別の待ち合わせ場所に
//! なる。

use crate::security;

/// 辞書サーバのパイプ。
pub fn pipe() -> String {
    format!(r"\\.\pipe\CrystalSKK.{}", user())
}

/// サーバが一つだけであることを示すミューテックス。
///
/// `Local\` を付けてログオンセッションの中に閉じる。
pub fn mutex() -> String {
    format!(r"Local\CrystalSKK.server.{}", user())
}

/// 名前に入れる利用者の印。
///
/// 調べられなければ、せめて衝突しない何かにする。**名前が無いよりは、
/// 共有されないほうがましである。**
fn user() -> String {
    security::current_user_sid().unwrap_or_else(|_| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipe_lives_in_the_pipe_namespace() {
        assert!(pipe().starts_with(r"\\.\pipe\"));
    }

    #[test]
    fn both_names_carry_the_same_user() {
        let user = user();
        assert!(pipe().ends_with(&user));
        assert!(mutex().ends_with(&user));
    }

    #[test]
    fn the_mutex_is_scoped_to_the_session() {
        // `Global\` にすると、別のログオンセッションと取り合う。
        assert!(mutex().starts_with(r"Local\"));
    }
}
