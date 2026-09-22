//! 辞書の置き場所。
//!
//! 利用者ごとの領域に置く。ユーザー辞書は書き換えるため管理者権限なしで
//! 書ける場所でなければならず、静的辞書もそれに揃えたほうが探しやすい。
//!
//! 導入した DLL とは別の場所である (DLL は `%ProgramFiles%`)。辞書は
//! 利用者のデータであって、プログラムの一部ではない。

use std::io;
use std::path::PathBuf;

/// 辞書と設定を置く場所。`%LOCALAPPDATA%\CrystalSKK`。
pub fn data_dir() -> io::Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| io::Error::other("LOCALAPPDATA が設定されていません"))?;
    Ok(PathBuf::from(base).join("CrystalSKK"))
}

/// 静的辞書の置き場所。
pub fn system_dictionary() -> io::Result<PathBuf> {
    Ok(data_dir()?.join(SYSTEM_DICTIONARY_NAME))
}

/// ユーザー辞書の置き場所。
pub fn user_dictionary() -> io::Result<PathBuf> {
    Ok(data_dir()?.join(USER_DICTIONARY_NAME))
}

/// 静的辞書の名前。取得元が何であれこの名前で置く。
pub const SYSTEM_DICTIONARY_NAME: &str = "SKK-JISYO.L";

/// ユーザー辞書の名前。
pub const USER_DICTIONARY_NAME: &str = "user.dict";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dictionaries_sit_together_under_the_data_directory() {
        let directory = data_dir().expect("LOCALAPPDATA がある");
        assert!(directory.ends_with("CrystalSKK"));
        assert_eq!(
            system_dictionary().expect("求まる").parent(),
            Some(directory.as_path())
        );
        assert_eq!(
            user_dictionary().expect("求まる").parent(),
            Some(directory.as_path())
        );
    }
}
