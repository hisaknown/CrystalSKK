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

use std::io;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::core::HSTRING;

use crate::log;

/// 窓を出さずに起こす。
///
/// **黙って立ち上がるべきものが窓を開くと、入力の邪魔になる。** 辞書
/// サーバは利用者が見るものではない。
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 実行ファイルの名前。
const SERVER_NAME: &str = "crystalskk-server.exe";

/// 辞書サーバを起こす。**起きるのを待たずに返る。**
///
/// 返るのは「起きてくる見込みがある」かどうか。すでに立ち上がりかけて
/// いれば (ログオンで起こしたものが辞書を読んでいる最中など)、もう一つ
/// 起こしても名乗りを上げられずに終わるだけなので、起こさない。CorvusSKK も
/// 同じ確かめ方をしている。
pub fn server() -> bool {
    if is_starting() {
        return true;
    }
    let Some(exe) = server_path() else {
        log::error("辞書サーバの場所が分かりません");
        return false;
    };
    if !exe.is_file() {
        log::error(&format!("辞書サーバがありません: {}", exe.display()));
        return false;
    }

    match Command::new(&exe).creation_flags(CREATE_NO_WINDOW).spawn() {
        Ok(_) => true,
        Err(e) => {
            // 隔離された入れ物の中ではここに来る。**失敗はするが、
            // 騒ぐことではない。** 導入とログオンで立っているのが普通で、
            // 立っていなければ利用者に伝える。
            log::error(&format!("辞書サーバを起こせません: {e}"));
            false
        }
    }
}

/// サーバがすでに名乗りを上げているか。
///
/// サーバは起きるとまず名乗り (ミューテックス) を上げ、辞書を読んでから
/// 待ち合わせ場所 (パイプ) を開く。パイプが無くても名乗りがあれば、
/// 起きかけている。
///
/// 隔離された入れ物の中からは名乗りが見えない (名前の置き場所が別になる)
/// ので、常に偽になる。そこでは起こすこともできないので、差し支えない。
fn is_starting() -> bool {
    let name = HSTRING::from(crystalskk_server::names::mutex());
    // SAFETY: 名前はここで用意したもの。開けたら閉じる。
    unsafe {
        match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &name) {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                true
            }
            Err(_) => false,
        }
    }
}

/// 起こしたサーバが待ち合わせ場所を開くまで、`attempt` を試し直す。
///
/// 試し直すのは**居ない** ([`io::ErrorKind::NotFound`]) ときだけである。
/// 居るのに答えないのなら、待っても直らないうえ、一度ごとに期限いっぱい
/// 待たされる。
///
/// 待ち合わせ場所が無ければ繋ぎに行ってもすぐ断られるので、短い間隔で
/// 叩いても軽い。**決め打ちで寝かせない**のは、早く起きればすぐ抜け、
/// 起きなければ上限で諦めるため。
pub fn wait_until_up<T>(mut attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let deadline = Instant::now() + STARTUP_WAIT;
    loop {
        std::thread::sleep(POLL_INTERVAL);
        match attempt() {
            Err(e) if e.kind() == io::ErrorKind::NotFound && Instant::now() < deadline => {}
            outcome => return outcome,
        }
    }
}

/// 辞書サーバの場所。**この DLL の隣に置いてある。**
fn server_path() -> Option<PathBuf> {
    let module = crate::registry::module_path(crate::module()).ok()?;
    let directory = PathBuf::from(module).parent()?.to_path_buf();
    Some(directory.join(SERVER_NAME))
}

/// 起こしてから待ち合わせ場所が開くのを待つ上限 (ADR-0032)。
///
/// ここを通るのはログオン直後や入れ替えの最中くらいで、サーバが本当に
/// 落ちることはまずない。**めったに払わないので、起きるのを待ってあげる
/// ほうを採る。** 間に合わなければ、次に頼んだときに繋がる。
const STARTUP_WAIT: Duration = Duration::from_millis(1000);

/// 待ち合わせ場所が開いたかを見る間隔。
const POLL_INTERVAL: Duration = Duration::from_millis(20);
