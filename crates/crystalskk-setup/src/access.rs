//! 隔離された入れ物から辞書を読めるようにする。
//!
//! TIP はストアアプリやスタートメニューの検索欄の中でも動く。そこは
//! AppContainer という隔離された入れ物で、**中からは利用者の領域が既定では
//! 読めない**。辞書が見えず、変換が一件も引けないまま終わる。
//!
//! CorvusSKK の作者もこう書いている。
//!
//! > AppContainerの外とファイル読み書きやプロセス間通信をおこなう必要が
//! > あるIMEは、予め設定ファイルなどのオブジェクトに対してAppContainer内から
//! > 読んでもいいよ、という状態に設定しておきます
//!
//! その「読んでもいいよ」を与えるのがこのモジュールである。
//!
//! # 読みだけを許す
//!
//! 書き込みは許さない。**許せば、隔離された入れ物で動くあらゆるアプリが
//! 利用者の辞書を書き換えられる**ことになる。隔離されている意味が薄れる。
//!
//! 代償として、そうした場所では学習が残らない。変換はできるが、選んだ
//! 候補の順番は覚えられない。本来の答えは辞書を別プロセスに持たせること
//! (PRD §7) で、そちらができるまでの妥協である。

use std::io;
use std::path::Path;

use windows::Win32::Foundation::LocalFree;
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{HSTRING, PWSTR};

/// 辞書の置き場所に、隔離された入れ物からの読みを許す。
///
/// 折り返しのために、中のファイルへ受け継がれる形で与える。あとから
/// 置かれる辞書にも、同じ許可が付く。
pub fn allow_app_containers(directory: &Path) -> io::Result<()> {
    let sddl = descriptor_text()?;
    apply(directory, &sddl)
}

/// 与える許可を SDDL で書き表す。
///
/// | 相手 | 許す範囲 |
/// |---|---|
/// | `AC` すべてのアプリケーションパッケージ | 読みと、フォルダをたどること |
/// | 制限されたすべてのパッケージ | 同上 |
/// | `SY` システム | すべて |
/// | `BA` 管理者 | すべて |
/// | 利用者自身 | すべて |
///
/// 権利は連結して書く (`FR` 読み + `FX` たどる)。フォルダは「たどる」が
/// 無いと中のファイルへ届かない。
///
/// 二つ目は SID をそのまま書く。**SDDL の略号 `RC` は別物**で、
/// 「制限されたコード」に解決されてしまう。Chromium 系の描画プロセスの
/// ように、より強く絞られた入れ物 (LPAC) で動くものはこちらに当たる。
///
/// `OICI` は「中のファイルとフォルダにも受け継ぐ」という印。`P` は
/// 受け継いできたものを断ち切る印で、ここに書いた分だけが効く。
///
/// **利用者自身を必ず入れる。** 断ち切ったうえで書き忘れると、自分の
/// 辞書を自分で読めなくなる。
fn descriptor_text() -> io::Result<String> {
    let user = current_user_sid()?;
    // 「制限されたすべてのアプリケーションパッケージ」。略号が無いので
    // SID を直に書く。`%ProgramFiles%` の既定にも同じものが入っている。
    const LPAC: &str = "S-1-15-2-2";
    Ok(format!(
        "D:P(A;OICI;FRFX;;;AC)(A;OICI;FRFX;;;{LPAC})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{user})"
    ))
}

/// いま動いている利用者の SID を文字列で得る。
fn current_user_sid() -> io::Result<String> {
    // SAFETY: 自分のプロセスの token を開き、この関数の中で閉じる。
    unsafe {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| io::Error::other(format!("利用者を調べられません: {}", e.message())))?;
        let token = Handle(token);

        // まず必要な大きさを尋ね、それから受け取る。
        let mut size = 0;
        let _ =
            windows::Win32::Security::GetTokenInformation(token.0, TokenUser, None, 0, &mut size);
        let mut buffer = vec![0u8; size as usize];
        windows::Win32::Security::GetTokenInformation(
            token.0,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            size,
            &mut size,
        )
        .map_err(|e| io::Error::other(format!("利用者を調べられません: {}", e.message())))?;

        let user = buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = PWSTR::null();
        ConvertSidToStringSidW((*user).User.Sid, &mut text)
            .map_err(|e| io::Error::other(format!("利用者を調べられません: {}", e.message())))?;
        let owned = text.to_string().map_err(io::Error::other);
        let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            text.as_ptr().cast(),
        )));
        owned
    }
}

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

        let status = SetNamedSecurityInfoW(
            &wide,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
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

/// 閉じ忘れないための持ち手。
struct Handle(windows::Win32::Foundation::HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: 自分で開いた token をここで閉じる。
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
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
