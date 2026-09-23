//! 誰に触らせるかを決める。
//!
//! パイプは待ち合わせ場所であり、**隔離された入れ物の中の TIP からも
//! 届かなければ意味がない**。ストアアプリで変換できなくなる。
//!
//! 辞書のファイルに読みを許したのと同じ話が、カーネルオブジェクトにも
//! 出てくる (ADR-0014, ADR-0016)。

use std::io;

use windows::Win32::Foundation::{HANDLE, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{HSTRING, PWSTR};

/// 「制限されたすべてのアプリケーションパッケージ」。
///
/// 略号が無いので SID を直に書く。SDDL の `RC` は「制限されたコード」と
/// いう別物である。
const LPAC: &str = "S-1-15-2-2";

/// いま動いている利用者の SID を文字列で得る。
pub fn current_user_sid() -> io::Result<String> {
    // SAFETY: 自分のプロセスの token を開き、この関数の中で閉じる。
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| io::Error::other(format!("利用者を調べられません: {}", e.message())))?;
        let token = OwnedHandle(token);

        // まず必要な大きさを尋ね、それから受け取る。
        let mut size = 0;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut size);
        let mut buffer = vec![0u8; size as usize];
        GetTokenInformation(
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

/// パイプに与える許可。
///
/// | 相手 | 許す範囲 |
/// |---|---|
/// | 利用者自身 | すべて |
/// | `SY` システム / `BA` 管理者 | すべて |
/// | `AC` すべてのアプリケーションパッケージ | 読み書きと同期 |
/// | 制限されたすべてのパッケージ | 同上 |
///
/// 隔離された入れ物には**すべてを与えない**。頼みを書いて答えを読めれば
/// 足りる。許可そのものを書き換える権利まで渡す理由はない。
///
/// 末尾の `S:(ML;;NW;;;LW)` は「低い整合性レベルからも書ける」という印。
/// 隔離された入れ物は低い整合性で動くので、これが無いと届かない。
fn descriptor_text(user: &str) -> String {
    format!(
        "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{user})(A;;GRGWGX;;;AC)(A;;GRGWGX;;;{LPAC})S:(ML;;NW;;;LW)"
    )
}

/// パイプを作るときに渡す許可。
///
/// 返った持ち手が生きている間だけ、中の記述子が有効である。
#[derive(Debug)]
pub struct PipeSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

impl PipeSecurity {
    pub fn new() -> io::Result<Self> {
        let user = current_user_sid()?;
        let sddl = HSTRING::from(descriptor_text(&user));

        // SAFETY: 組み立てた記述子は `Drop` で返す。
        let descriptor = unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &sddl,
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
            .map_err(|e| io::Error::other(format!("許可を組み立てられません: {}", e.message())))?;
            descriptor
        };

        let attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        Ok(Self {
            descriptor,
            attributes,
        })
    }

    /// `CreateNamedPipeW` へ渡す形。
    pub fn attributes(&self) -> *const SECURITY_ATTRIBUTES {
        std::ptr::from_ref(&self.attributes)
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        // SAFETY: `ConvertStringSecurityDescriptorToSecurityDescriptorW` が
        // 返したものは `LocalFree` で返す約束になっている。
        unsafe {
            let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(self.descriptor.0)));
        }
    }
}

/// 閉じ忘れないための持ち手。
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: 自分で開いたものをここで閉じる。
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_can_be_identified() {
        let sid = current_user_sid().expect("自分の SID は分かる");
        assert!(sid.starts_with("S-1-"), "SID の形をしている: {sid}");
    }

    #[test]
    fn app_containers_get_less_than_everything() {
        let sddl = descriptor_text("S-1-5-21-0-0-0-1000");
        assert!(sddl.contains("(A;;GRGWGX;;;AC)"), "読み書きだけ: {sddl}");
        assert!(!sddl.contains("(A;;GA;;;AC)"), "すべては与えない: {sddl}");
    }

    #[test]
    fn low_integrity_callers_can_reach_it() {
        // これが無いと、隔離された入れ物から書き込めない。
        assert!(descriptor_text("S-1-5-21-0-0-0-1000").contains("S:(ML;;NW;;;LW)"));
    }

    #[test]
    fn the_descriptor_is_accepted_by_windows() {
        // 書き方を間違えていれば、ここで組み立てに失敗する。
        PipeSecurity::new().expect("許可を組み立てられる");
    }
}
