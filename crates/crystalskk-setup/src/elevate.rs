//! 管理者権限への昇格。
//!
//! 入力方式の登録は機械全体に書かれるため、管理者権限が要る (ADR-0007)。
//! 利用者に別のシェルを開き直させるのは不親切なので、自分を昇格して
//! 呼び直す。UAC の確認がそのまま同意になる。
//!
//! 昇格した側は別のコンソールで動くため、その出力は元のコンソールに
//! 出ない。そこで、伝えたいことは一時ファイルに書かせて読み戻す。

use std::io;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{HSTRING, PCWSTR, w};

/// いま管理者権限で動いているか。
pub fn is_elevated() -> bool {
    let mut token = HANDLE::default();
    // SAFETY: 出力先のハンドルは有効な場所を指す。失敗したら偽を返す。
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened.is_err() {
        return false;
    }

    let mut elevation = TOKEN_ELEVATION::default();
    let mut size = 0u32;
    // SAFETY: 構造体の大きさを正しく伝えており、書き込み先も有効。
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(std::ptr::from_mut(&mut elevation).cast()),
            u32::try_from(size_of::<TOKEN_ELEVATION>()).unwrap_or(0),
            &mut size,
        )
    };
    // SAFETY: 直前に開いたハンドルを閉じる。
    unsafe {
        let _ = CloseHandle(token);
    }

    queried.is_ok() && elevation.TokenIsElevated != 0
}

/// 自分を管理者権限で呼び直し、終わるまで待つ。
///
/// 戻り値は呼び直した側の終了コード。UAC の確認を断られた場合は失敗する。
pub fn relaunch_as_administrator(arguments: &[String]) -> io::Result<u32> {
    let exe = std::env::current_exe()?;
    let command_line = arguments
        .iter()
        .map(|a| quote(a))
        .collect::<Vec<_>>()
        .join(" ");

    let file = HSTRING::from(exe.as_os_str());
    let parameters = HSTRING::from(command_line.as_str());
    let directory = std::env::current_dir().ok();
    let directory = directory.as_deref().map(Path::as_os_str).map(HSTRING::from);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: u32::try_from(size_of::<SHELLEXECUTEINFOW>()).unwrap_or(0),
        fMask: SEE_MASK_NOCLOSEPROCESS,
        // `runas` が UAC の確認を出す。
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        lpDirectory: directory
            .as_ref()
            .map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: SW_HIDE.0,
        ..Default::default()
    };

    // SAFETY: 構造体は上で埋めており、指している文字列はこの関数の間生きている。
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| {
        io::Error::other(format!(
            "管理者権限での実行を開始できません: {}",
            e.message()
        ))
    })?;

    if info.hProcess.is_invalid() {
        return Err(io::Error::other("管理者権限の処理を追跡できません"));
    }

    // SAFETY: 有効なプロセスハンドルを待ち、終了コードを読んで閉じる。
    let code = unsafe {
        let waited = WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 0u32;
        let read = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if waited != WAIT_OBJECT_0 || read.is_err() {
            return Err(io::Error::other("管理者権限の処理の結果を読めません"));
        }
        code
    };
    Ok(code)
}

/// 空白を含む引数を包む。
fn quote(argument: &str) -> String {
    if argument.contains([' ', '\t']) {
        format!("\"{}\"", argument.replace('"', "\\\""))
    } else {
        argument.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_only_what_needs_it() {
        assert_eq!(quote("install"), "install");
        assert_eq!(
            quote(r"C:\Program Files\a.dll"),
            "\"C:\\Program Files\\a.dll\""
        );
    }
}
