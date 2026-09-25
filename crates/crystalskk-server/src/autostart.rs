//! ログオンのたびに辞書サーバを起こす登録。
//!
//! TIP は、居なければ自分でサーバを起こす。起こせないのは隔離された
//! アプリの中だけで、自動起動はその場面のためにある。
//!
//! **登録はサーバ自身が、起きたときに行う** (ADR-0037)。CrystalSKK を
//! 使った利用者にだけ効き、使わない利用者に辞書と言語モデルを抱えた
//! プロセスを常駐させない。利用者ごとの場所に書くので、権限も要らない。

use std::path::Path;

use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{HSTRING, Result};

/// ログオンのたびに起きるものが並ぶ場所。
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// 登録に使う名前。
const RUN_NAME: &str = "CrystalSKK";

/// `exe` を自動起動に登録する。同じものがすでに書かれていれば何もしない。
pub fn register(exe: &Path) -> Result<()> {
    let command = command(exe);
    if registered().as_deref() == Some(command.as_str()) {
        return Ok(());
    }
    // REG_SZ は終端の NUL を含めた長さで書く。
    let data: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
    let length = u32::try_from(std::mem::size_of_val(&data[..])).unwrap_or(u32::MAX);
    // SAFETY: 書く中身はこの関数の変数で、長さも渡している。
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(RUN_NAME),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            length,
        )
    }
    .ok()
}

/// 自動起動の登録を消す。無ければそれでよい。
pub fn unregister() -> Result<()> {
    // SAFETY: 根のハンドルは定数で、名前は有効な文字列。
    let status = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(RUN_NAME),
        )
    };
    // 無いものを消そうとしただけなら、それでよい (2 = ERROR_FILE_NOT_FOUND)。
    if status.0 == 2 { Ok(()) } else { status.ok() }
}

/// いま書かれている起動の指定。
fn registered() -> Option<String> {
    let key = HSTRING::from(RUN_KEY);
    let name = HSTRING::from(RUN_NAME);
    let mut buffer = [0u16; 1024];
    let mut size = u32::try_from(std::mem::size_of_val(&buffer)).unwrap_or(0);
    // SAFETY: 出力先はこの関数の配列で、大きさも渡している。
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&raw mut size),
        )
    }
    .ok()
    .ok()?;
    let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length]))
}

fn command(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_is_quoted_for_the_spaces_in_program_files() {
        assert_eq!(
            command(Path::new(
                r"C:\Program Files\CrystalSKK\bin\crystalskk-server.exe"
            )),
            r#""C:\Program Files\CrystalSKK\bin\crystalskk-server.exe""#
        );
    }
}
