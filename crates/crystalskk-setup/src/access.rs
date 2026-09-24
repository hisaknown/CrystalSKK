//! 辞書に与えた許可を取り下げる。
//!
//! かつて、隔離された入れ物 (AppContainer) の中の TIP に辞書を読ませる
//! ため、辞書のファイルへ読みを与えていた (ADR-0014)。
//!
//! **辞書サーバができて、それが要らなくなった** (ADR-0016)。読むのは
//! サーバだけで、サーバは隔離された入れ物の外にいる。TIP はファイルに
//! 触れない。
//!
//! 要らなくなった許可は**外す**。与えた相手は「あらゆる隔離されたアプリ」
//! という集団で、その中には利用者の辞書を覗く理由のないものしかいない。
//! とくに `user.dict` には**利用者が登録し、学習した語**が入る。
//!
//! # 与えたものは、こちらで外す
//!
//! 新しく入れる人には関係がないが、**すでに与えてしまった機械がある**。
//! 放っておけば残り続けるので、導入のたびに外す。

use std::io;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::LocalFree;
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_FILE_OBJECT,
    SetNamedSecurityInfoW,
};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PSECURITY_DESCRIPTOR,
    UNPROTECTED_DACL_SECURITY_INFORMATION,
};
use windows::core::HSTRING;

/// かつて与えた許可を取り下げる。
///
/// 置き場所とファイルのどちらからも、こちらで足した分を落とす。受け継いだ
/// 分は残るので、**利用者自身は今までどおり読み書きできる**。
///
/// 与えていない機械では何も変わらない。
pub fn revoke_app_containers(directory: &Path, files: &[PathBuf]) -> io::Result<()> {
    // 受け継ぎを断ち切っていたので、まず戻す。戻せば `%LOCALAPPDATA%`
    // からの許可が流れてきて、利用者・システム・管理者の分が揃う。
    apply(directory, INHERIT_AGAIN)?;

    for file in files {
        if !file.is_file() {
            continue;
        }
        apply(file, INHERIT_AGAIN)?;
    }
    Ok(())
}

/// 明示した許可を空にする印。
///
/// 受け継ぎを断ち切らないので、親から流れてくる分だけが残る。**こちらが
/// 足したものだけが消える。**
const INHERIT_AGAIN: &str = "D:";

/// 書き表した許可を、その場所に与える。
fn apply(path: &Path, sddl: &str) -> io::Result<()> {
    let wide = HSTRING::from(path.as_os_str());
    let sddl = HSTRING::from(sddl);

    // SAFETY: 組み立てた記述子はこの関数の中で解放する。
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &sddl,
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(|e| io::Error::other(format!("許可を組み立てられません: {}", e.message())))?;
        let descriptor = LocalOwned(descriptor);

        let mut present = windows::core::BOOL::default();
        let mut dacl = std::ptr::null_mut();
        let mut defaulted = windows::core::BOOL::default();
        GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted)
            .map_err(|e| io::Error::other(format!("許可を読み出せません: {}", e.message())))?;

        // **受け継ぎを必ず戻す。** 断ち切ったままだと、親から流れてくる
        // 利用者・システム・管理者の分まで失われる。
        let status = SetNamedSecurityInfoW(
            &wide,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        );
        if status.is_err() {
            return Err(io::Error::other(format!(
                "{} に許可を与えられません: {}",
                path.display(),
                status.to_hresult().message()
            )));
        }
    }
    Ok(())
}

/// `LocalFree` で返す持ち手。
struct LocalOwned(PSECURITY_DESCRIPTOR);

impl Drop for LocalOwned {
    fn drop(&mut self) {
        // SAFETY: `ConvertStringSecurityDescriptorToSecurityDescriptorW` が
        // 返したものは `LocalFree` で返す約束になっている。
        unsafe {
            let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(self.0.0)));
        }
    }
}
