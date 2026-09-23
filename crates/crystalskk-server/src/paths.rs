//! 辞書の置き場所。
//!
//! 利用者ごとの領域に置く。ユーザー辞書は書き換えるため管理者権限なしで
//! 書ける場所でなければならず、静的辞書もそれに揃えたほうが探しやすい。
//!
//! 導入した DLL とは別の場所である (DLL は `%ProgramFiles%`)。辞書は
//! 利用者のデータであって、プログラムの一部ではない。
//!
//! # 環境変数では足りない
//!
//! TIP は隔離された入れ物 (AppContainer) の中でも動く。ストアアプリや
//! スタートメニューの検索欄がそれで、**そこでは `%LOCALAPPDATA%` が入れ物
//! ごとの場所を指す**。辞書はそちらには無いので、引けずに終わる。
//!
//! そこで置き場所は環境変数ではなく Win32 に尋ね、**入れ物への読み替えを
//! しないよう明示する** (`KF_FLAG_NO_APPCONTAINER_REDIRECTION`)。こうすると
//! どこから呼んでも同じ場所を指す。
//!
//! ただし場所が分かっても、**入れ物の中から読めるとは限らない**。ファイル
//! の側にも許可が要る。そちらは導入のときに与える。

use std::io;
use std::path::PathBuf;

/// 辞書と設定を置く場所。`%LOCALAPPDATA%\CrystalSKK`。
pub fn data_dir() -> io::Result<PathBuf> {
    Ok(local_app_data()?.join("CrystalSKK"))
}

/// URL から取ってきた辞書を置く場所。
///
/// 中は取得元ごとに分かれる (`raw.githubusercontent.com\skk-dev\…`)。
pub fn dictionary_cache() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("dictionaries"))
}

/// かつて静的辞書を置いていた場所。
///
/// 辞書を設定で選べるようにする前は、L 辞書をここに一つだけ置いていた
/// (ADR-0022)。いまは読まない。導入のときに片付ける。
pub fn system_dictionary() -> io::Result<PathBuf> {
    Ok(data_dir()?.join(SYSTEM_DICTIONARY_NAME))
}

/// ユーザー辞書の置き場所。
pub fn user_dictionary() -> io::Result<PathBuf> {
    Ok(data_dir()?.join(USER_DICTIONARY_NAME))
}

/// 設定ファイルの置き場所。
///
/// 辞書と並べる。**利用者が開いて書き換えるファイル**なので、探しやすい
/// 場所にまとめておく。
pub fn settings() -> io::Result<PathBuf> {
    Ok(data_dir()?.join(SETTINGS_NAME))
}

/// 設定ファイルの名前。
pub const SETTINGS_NAME: &str = "config.toml";

/// 静的辞書の名前。取得元が何であれこの名前で置く。
pub const SYSTEM_DICTIONARY_NAME: &str = "SKK-JISYO.L";

/// ユーザー辞書の名前。
pub const USER_DICTIONARY_NAME: &str = "user.dict";

/// 利用者ごとの領域。
///
/// 隔離された入れ物の中から呼ばれても、**本物の場所**を返す。
fn local_app_data() -> io::Result<PathBuf> {
    use windows::Win32::UI::Shell::{
        FOLDERID_LocalAppData, KF_FLAG_DONT_VERIFY, KF_FLAG_NO_APPCONTAINER_REDIRECTION,
        SHGetKnownFolderPath,
    };

    // SAFETY: 定数の識別子で尋ね、返された文字列はここで写して解放する。
    let path = unsafe {
        let raw = SHGetKnownFolderPath(
            &FOLDERID_LocalAppData,
            KF_FLAG_NO_APPCONTAINER_REDIRECTION | KF_FLAG_DONT_VERIFY,
            None,
        )
        .map_err(|e: windows::core::Error| {
            io::Error::other(format!("置き場所を尋ねられません: {}", e.message()))
        })?;
        let owned = raw.to_string().map_err(io::Error::other);
        windows::Win32::System::Com::CoTaskMemFree(Some(raw.as_ptr().cast()));
        owned?
    };
    Ok(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dictionaries_sit_together_under_the_data_directory() {
        let directory = data_dir().expect("置き場所が求まる");
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

    #[test]
    fn the_place_is_the_real_one_not_a_containers_copy() {
        // 隔離された入れ物の中では `%LOCALAPPDATA%` が読み替えられる。
        // ここは普通のプロセスなので、環境変数と一致するはずである。
        // **一致しないなら、尋ね方が間違っている。**
        let asked = data_dir().expect("置き場所が求まる");
        let Some(from_variable) = std::env::var_os("LOCALAPPDATA") else {
            return;
        };
        assert_eq!(asked, PathBuf::from(from_variable).join("CrystalSKK"));
    }
}
