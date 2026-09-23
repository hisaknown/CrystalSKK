//! 辞書サーバの入れ替えと起動。
//!
//! サーバは**ユーザー辞書の唯一の書き手**である。入れ替えるとき、落として
//! しまうと書きかけの学習が消える。だから頼んで終わってもらう。
//!
//! # 手順
//!
//! 1. 終わってくれと頼む (`exit`)
//! 2. 消えるのを待つ
//! 3. 新しいものを置く
//! 4. 起こす
//!
//! 消えてくれないときは、**退ける**。動いている exe は上書きできないが
//! 改名はできる。DLL でやっているのと同じ手で、そちらは ADR-0007 の
//! 帰結として入れてある。

use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crystalskk_ipc::Request;
use crystalskk_server::client;

/// 実行ファイルの名前。
pub const SERVER_NAME: &str = "crystalskk-server.exe";

/// 窓を出さずに起こすための印。
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 終わるのを待つ上限。
const GIVE_UP_AFTER: Duration = Duration::from_secs(5);

/// 様子を見る間隔。
const CHECK_EVERY: Duration = Duration::from_millis(100);

/// 動いているサーバに終わってもらう。
///
/// 返るのは「終わってもらった」かどうか。動いていなければ `false`。
pub fn stop() -> bool {
    if client::ask(&Request::Exit).is_err() {
        return false;
    }

    // 応答は「聞き届けた」であって「もう居ない」ではない。消えるまで待つ。
    let deadline = Instant::now() + GIVE_UP_AFTER;
    while Instant::now() < deadline {
        if !client::is_running() {
            return true;
        }
        std::thread::sleep(CHECK_EVERY);
    }
    // 消えなくても構わない。置き換えは退ける手で通る。
    true
}

/// サーバを起こす。
///
/// 導入した直後に呼ぶ。**ログオンし直すまで待たせない**ためで、最初に
/// 開いたのがストアアプリでも変換できるようにしたい。隔離された入れ物の
/// 中からはプロセスを起こせないので、ここで起こしておく意味がある。
pub fn start(directory: &Path) -> io::Result<()> {
    let exe = directory.join(SERVER_NAME);
    if !exe.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} がありません", exe.display()),
        ));
    }
    // 窓を出さない。**黙って立ち上がるべきものが窓を開くと、利用者は
    // 「何か始まった」と身構える。**
    std::process::Command::new(&exe)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    Ok(())
}

/// ログオンのたびに起きるよう登録する。
///
/// 隔離されたアプリからはプロセスを起こせない。**最初に開いたのがストア
/// アプリだと、誰も起こせないまま変換できない**ことになる。ログオンで
/// 立てておけば、その場面が起きにくくなる。
pub fn register_autostart(directory: &Path) -> io::Result<()> {
    let exe = directory.join(SERVER_NAME);
    crystalskk_tip::registry::write_run_entry(RUN_NAME, &format!("\"{}\"", exe.display()))
        .map_err(|e| io::Error::other(format!("自動起動を登録できません: {}", e.message())))
}

/// 自動起動の登録を消す。
pub fn unregister_autostart() {
    let _ = crystalskk_tip::registry::delete_run_entry(RUN_NAME);
}

/// 自動起動に書く名前。
const RUN_NAME: &str = "CrystalSKK";

/// ビルド成果物の中からサーバを探す。
///
/// DLL と同じ考え方で、release を先に見る。
pub fn default_source() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("target/release").join(SERVER_NAME)];
    if let Ok(exe) = std::env::current_exe()
        && let Some(directory) = exe.parent()
    {
        candidates.push(directory.join(SERVER_NAME));
    }
    candidates.push(PathBuf::from("target/debug").join(SERVER_NAME));
    candidates.into_iter().find(|path| path.is_file())
}
