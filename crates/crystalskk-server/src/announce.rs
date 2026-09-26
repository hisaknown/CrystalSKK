//! 設定ファイルが変わったと、全ウィンドウへ知らせる (ADR-0040)。
//!
//! TIP はアプリの数だけいて、それぞれが設定の写しを持つ。写しが古く
//! なったことを TIP は自分では知れないので、持ち主のこちらが知らせる。
//!
//! 知らせるのは Windows 自身が設定の変化を `WM_SETTINGCHANGE` で知らせる
//! のと同じ形で、名前から番号を振った独自のメッセージを一斉に送る。
//! **読むのはこちらではない。** 知らされた TIP が、いつもどおり設定を
//! 尋ねてくる。サーバ自身の辞書やランカーもそのときに合わせる。

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, PostMessageW, RegisterWindowMessageW,
};
use windows::core::{HSTRING, PCWSTR};

/// 一度の保存で、エディタは何度も書く。まとめて一度だけ知らせるための間。
const SETTLE: Duration = Duration::from_millis(200);

/// 全ウィンドウへ知らせる。相手を待たない。
///
/// 固まっているアプリがあっても、こちらは止まらない。相手は動き出したときに
/// 受け取る。
pub fn announce() -> io::Result<()> {
    let name = HSTRING::from(crystalskk_ipc::SETTINGS_CHANGED_MESSAGE);
    // SAFETY: 終端のある名前を渡している。
    let message = unsafe { RegisterWindowMessageW(&name) };
    if message == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 全体へ送るだけで、何も預けない。
    unsafe { PostMessageW(Some(HWND_BROADCAST), message, WPARAM(0), LPARAM(0)) }
        .map_err(io::Error::from)
}

/// 設定ファイルの置き場所を見張り始める。裏のスレッドで動き続ける。
///
/// 見張れなくても、サーバは動き続ける。書き換えた設定は、入力先を
/// 切り替えたときか、「設定を検査する」で効く。
pub fn watch(settings: PathBuf) {
    std::thread::spawn(move || {
        if let Err(e) = watch_forever(&settings) {
            eprintln!("crystalskk-server: 設定ファイルを見張れません: {e}");
        }
    });
}

fn watch_forever(settings: &Path) -> io::Result<()> {
    let directory = settings
        .parent()
        .ok_or_else(|| io::Error::other("設定ファイルの置き場所が分かりません"))?;
    let handle = open_directory(directory)?;
    // `FILE_NOTIFY_INFORMATION` は 4 バイト境界に並ぶ。u32 の配列で受ける。
    let mut buffer = vec![0u32; 16 * 1024];
    loop {
        let mut returned = 0u32;
        // SAFETY: 開いた場所と、長さ付きの受け皿を渡している。重ねない (同期) 呼び出し。
        unsafe {
            ReadDirectoryChangesW(
                handle.0,
                buffer.as_mut_ptr().cast(),
                u32::try_from(buffer.len() * 4).unwrap_or(u32::MAX),
                false,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_LAST_WRITE
                    | FILE_NOTIFY_CHANGE_SIZE,
                Some(&mut returned),
                None,
                None,
            )?;
        }
        // 0 は受け皿があふれたということ。何が変わったか分からないので、
        // 知らせておく。
        let bytes: Vec<u8> = buffer.iter().flat_map(|word| word.to_ne_bytes()).collect();
        let changed = changed_names(&bytes[..returned as usize]);
        if returned == 0 || changed.iter().any(|name| watched(settings, name)) {
            std::thread::sleep(SETTLE);
            if let Err(e) = announce() {
                eprintln!("crystalskk-server: 設定の変化を知らせられません: {e}");
            }
        }
    }
}

/// 見張るファイルか。設定ファイルと、同じ場所にあるローマ字テーブル。
///
/// ほかのファイルでは知らせない。**同じ場所には TIP のログもあり、打鍵の
/// たびに書かれる。** ローマ字テーブルの名前は変わりうるので、そのつど
/// 設定ファイルから読む。
fn watched(settings: &Path, name: &OsString) -> bool {
    let same = |path: &Path| {
        path.file_name()
            .is_some_and(|own| own.eq_ignore_ascii_case(name))
    };
    if same(settings) {
        return true;
    }
    let Ok(text) = std::fs::read_to_string(settings) else {
        return false;
    };
    crystalskk_settings::romaji_path(settings, &text)
        .is_ok_and(|romaji| romaji.parent() == settings.parent() && same(&romaji))
}

/// 知らせの並びから、変わったファイルの名前を取り出す。
///
/// 一つの知らせは、次の知らせまでの距離、何が起きたか、名前の長さ (バイト)、
/// 名前 (UTF-16) の順に並ぶ。
fn changed_names(bytes: &[u8]) -> Vec<OsString> {
    let word = |at: usize| -> Option<u32> {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
    };
    let mut names = Vec::new();
    let mut at = 0;
    while let (Some(next), Some(length)) = (word(at), word(at + 8)) {
        let start = at + 12;
        let Some(name) = bytes.get(start..start + length as usize) else {
            break;
        };
        let units: Vec<u16> = name
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_ne_bytes(*pair))
            .collect();
        names.push(OsString::from_wide(&units));
        if next == 0 {
            break;
        }
        at += next as usize;
    }
    names
}

/// 見張るために開いた場所。落とせば閉じる。
struct Directory(HANDLE);

impl Drop for Directory {
    fn drop(&mut self) {
        // SAFETY: 開いた持ち手を一度だけ閉じる。
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn open_directory(directory: &Path) -> io::Result<Directory> {
    let wide: Vec<u16> = directory.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: 終端のある名前を渡している。場所を開くには BACKUP_SEMANTICS が要る。
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }?;
    Ok(Directory(handle))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 知らせの並びを一つ作る。
    fn entry(name: &str, last: bool) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let length = units.len() * 2;
        // 次の知らせは 4 バイト境界から始まる。
        let size = (12 + length).div_ceil(4) * 4;
        let mut bytes = Vec::new();
        bytes.extend(
            u32::try_from(if last { 0 } else { size })
                .unwrap()
                .to_ne_bytes(),
        );
        bytes.extend(3u32.to_ne_bytes());
        bytes.extend(u32::try_from(length).unwrap().to_ne_bytes());
        bytes.extend(units.iter().flat_map(|u| u.to_ne_bytes()));
        bytes.resize(size, 0);
        bytes
    }

    #[test]
    fn the_names_of_changed_files_are_read() {
        let mut bytes = entry("config.toml.writing", false);
        bytes.extend(entry("config.toml", true));
        assert_eq!(
            changed_names(&bytes),
            [
                OsString::from("config.toml.writing"),
                OsString::from("config.toml")
            ]
        );
    }

    #[test]
    fn a_broken_list_stops_quietly() {
        let mut bytes = entry("config.toml", true);
        bytes.truncate(14);
        assert!(changed_names(&bytes).is_empty());
    }

    #[test]
    fn only_the_settings_and_the_romaji_table_are_watched() {
        let directory =
            std::env::temp_dir().join(format!("crystalskk-watch-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let settings = directory.join("config.toml");
        std::fs::write(&settings, "[romaji]\ntable = \"kana.txt\"\n").unwrap();

        assert!(watched(&settings, &OsString::from("config.toml")));
        assert!(watched(&settings, &OsString::from("CONFIG.TOML")));
        assert!(watched(&settings, &OsString::from("kana.txt")));
        assert!(!watched(&settings, &OsString::from("tip.log")));
        assert!(!watched(&settings, &OsString::from("romaji.txt")));

        std::fs::remove_dir_all(&directory).unwrap();
    }
}
