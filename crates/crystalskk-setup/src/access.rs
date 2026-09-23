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
//! # 辞書だけを許す。置き場所ごと開けない
//!
//! 置き場所には辞書のほかに**診断の記録**も置いてある。記録は `trace` に
//! すると打鍵を残すので、辞書と同じ扱いで開くわけにはいかない。
//!
//! そこで、
//!
//! - **フォルダには「たどる」だけを許す。** 中を並べて見ることはできない
//! - **辞書のファイルにだけ「読む」を許す。** 名前を知っているものしか
//!   開けない
//!
//! 記録には何も与えない。隔離された入れ物から見れば、そこに無いのと同じ
//! になる。
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
use std::path::{Path, PathBuf};

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

/// 辞書だけを、隔離された入れ物から読めるようにする。
///
/// `readable` に挙げたファイルにだけ読みを許す。**置き場所そのものは
/// たどれるだけ**で、中に何があるかは見えない。
///
/// 挙げられていないファイル — 診断の記録など — には何も与えない。
pub fn allow_app_containers(directory: &Path, readable: &[PathBuf]) -> io::Result<()> {
    let user = current_user_sid()?;
    apply(directory, &directory_rules(&user), Protect::Yes)?;

    for file in readable {
        if !file.is_file() {
            continue;
        }
        // ファイルの側は受け継いだ分を残す。断ち切ると、自分の辞書を
        // 自分で読めなくなる。
        apply(file, FILE_RULES, Protect::No)?;
    }
    Ok(())
}

/// 受け継いできた許可を断ち切るか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Protect {
    /// 断ち切る。ここに書いた分だけが効く。
    Yes,
    /// 残す。書いた分は足されるだけ。
    No,
}

/// 「制限されたすべてのアプリケーションパッケージ」。
///
/// 略号が無いので SID を直に書く。**SDDL の略号 `RC` は別物**で、
/// 「制限されたコード」に解決されてしまう。Chromium 系の描画プロセスの
/// ように、より強く絞られた入れ物 (LPAC) で動くものはこちらに当たる。
const LPAC: &str = "S-1-15-2-2";

/// 置き場所に与える許可。
///
/// | 相手 | 許す範囲 | 受け継ぎ |
/// |---|---|---|
/// | `AC` すべてのアプリケーションパッケージ | **たどるだけ** | しない |
/// | 制限されたすべてのパッケージ | 同上 | しない |
/// | `SY` システム | すべて | する |
/// | `BA` 管理者 | すべて | する |
/// | 利用者自身 | すべて | する |
///
/// 隔離された入れ物に与えるのは `FX` (たどる) だけで、`FR` (読む) は
/// 与えない。**フォルダを読めると、中に何があるか並べて見られる。**
/// たどるだけなら、名前を知っているものしか開けない。
///
/// そして**受け継がせない**。受け継がせると、診断の記録まで読めるように
/// なる。
///
/// `OICI` は「中のファイルとフォルダにも受け継ぐ」という印。`P` は
/// 受け継いできたものを断ち切る印で、ここに書いた分だけが効く。
///
/// **利用者自身を必ず入れる。** 断ち切ったうえで書き忘れると、自分の
/// 辞書を自分で読めなくなる。
fn directory_rules(user: &str) -> String {
    format!("D:P(A;;FX;;;AC)(A;;FX;;;{LPAC})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{user})")
}

/// 辞書のファイルに足す許可。読みだけを与える。
const FILE_RULES: &str = "D:(A;;FR;;;AC)(A;;FR;;;S-1-15-2-2)";

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
fn apply(path: &Path, sddl: &str, protect: Protect) -> io::Result<()> {
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

        let information = match protect {
            Protect::Yes => DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            Protect::No => DACL_SECURITY_INFORMATION,
        };
        let status = SetNamedSecurityInfoW(
            &wide,
            SE_FILE_OBJECT,
            information,
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
