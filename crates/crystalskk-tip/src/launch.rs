//! 辞書サーバを起こす。
//!
//! 居なければ引けない。**利用者に「起こしてください」と言っても始まらない**
//! ので、まず自分で起こしてみる。CorvusSKK も引くたびに同じことをしている。
//!
//! # 起こせない場面がある
//!
//! 隔離された入れ物 (AppContainer) の中からは、プロセスを作れない。ストア
//! アプリやスタートメニューの検索欄がそれで、**そこで最初に使われると
//! 誰も起こせない**。
//!
//! だから導入とログオンでも起こしてある。ここは最後の手当てであって、
//! 唯一の手立てではない。

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use crate::log;

/// 窓を出さずに起こす。
///
/// **黙って立ち上がるべきものが窓を開くと、入力の邪魔になる。** 辞書
/// サーバは利用者が見るものではない。
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 実行ファイルの名前。
const SERVER_NAME: &str = "crystalskk-server.exe";

/// 辞書サーバを起こす。
///
/// 返るのは「起こしてみた」かどうか。起こせたかどうかまでは分からない
/// ので、呼んだ側が改めて尋ねる。
pub fn server() -> bool {
    let Some(exe) = server_path() else {
        log::error("辞書サーバの場所が分かりません");
        return false;
    };
    if !exe.is_file() {
        log::error(&format!("辞書サーバがありません: {}", exe.display()));
        return false;
    }

    match Command::new(&exe).creation_flags(CREATE_NO_WINDOW).spawn() {
        Ok(_) => {
            // 起きるまでの間を置く。読み込みに 100 ミリ秒ほどかかる。
            std::thread::sleep(std::time::Duration::from_millis(STARTUP_WAIT_MS));
            true
        }
        Err(e) => {
            // 隔離された入れ物の中ではここに来る。**失敗はするが、
            // 騒ぐことではない。** 導入とログオンで立っているのが普通で、
            // 立っていなければ利用者に伝える。
            log::error(&format!("辞書サーバを起こせません: {e}"));
            false
        }
    }
}

/// 辞書サーバの場所。**この DLL の隣に置いてある。**
fn server_path() -> Option<PathBuf> {
    let module = crate::registry::module_path(crate::module()).ok()?;
    let directory = PathBuf::from(module).parent()?.to_path_buf();
    Some(directory.join(SERVER_NAME))
}

/// 起こしてから尋ね直すまでの間。
///
/// 辞書を読むのに 100 ミリ秒ほどかかる。**打鍵の途中なので、長くは
/// 待てない。** 間に合わなければ次の変換で繋がる。
const STARTUP_WAIT_MS: u64 = 300;
