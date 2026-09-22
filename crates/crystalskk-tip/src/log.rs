//! 診断用の記録。
//!
//! TIP は他人のプロセスの中で動くので、標準出力もデバッガも当てにできない。
//! 何が起きているかを知る手段が要る。
//!
//! 記録先は `%LOCALAPPDATA%\CrystalSKK\tip.log`。利用者ごとの場所なので、
//! 機械全体に導入されていても書き込みに権限は要らない。
//!
//! 既定では何も書かない。環境変数 `CRYSTALSKK_LOG` が設定されているときだけ
//! 記録する。入力のたびにファイルを開くのは、常用してよい代物ではない。
//! **これは開発のための仕掛けであり、入力の速さを損なう** (PRD N-01)。
//!
//! # 「どのアプリの、いつの話か」を残す
//!
//! 一つのファイルに、あらゆるアプリの記録が混ざって流れ込む。番号だけでは
//! **どのアプリの記録か分からない**。番号は使い回されるうえ、後から調べよう
//! にもそのプロセスはもう居ない。
//!
//! そこで、読み込まれたときに一度だけ**実行ファイルの名前**を書き、以降の
//! 各行には**時刻**を添える。「この打鍵はあのアプリのものか」「今の操作で
//! 増えた行はどれか」が、これで言い当てられる。

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::SystemInformation::GetLocalTime;

/// 記録するかどうか。起動時に一度だけ決める。
static ENABLED: OnceLock<Option<PathBuf>> = OnceLock::new();

/// 記録先を決める。環境変数がなければ記録しない。
fn destination() -> Option<&'static PathBuf> {
    ENABLED
        .get_or_init(|| {
            std::env::var_os("CRYSTALSKK_LOG")?;
            let base = std::env::var_os("LOCALAPPDATA")?;
            let directory = PathBuf::from(base).join("CrystalSKK");
            std::fs::create_dir_all(&directory).ok()?;
            let path = directory.join("tip.log");
            announce(&path);
            Some(path)
        })
        .as_ref()
}

/// 読み込まれたことを、どのアプリの中かと共に書く。
///
/// [`destination`] の初期化の中から呼ぶ。[`write`] を使うと初期化が
/// 入れ子になるので、ここだけは自分でファイルを開く。
fn announce(path: &std::path::Path) {
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(
        file,
        "[{} {}] ===== 読み込まれた: {} =====",
        now(),
        std::process::id(),
        host()
    );
}

/// この記録を出しているアプリの実行ファイル。
fn host() -> String {
    let mut buffer = [0u16; 260];
    // SAFETY: 引数に `None` を渡すと、いま動いているプロセスの実行ファイルを
    // 指す。書き込み先は手元の配列で、長さもそのまま渡している。
    let length = unsafe { GetModuleFileNameW(None, &mut buffer) } as usize;
    if length == 0 {
        return "(分からない)".to_owned();
    }
    let path = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
    // 全部の道のりは長いので、名前だけにする。どのアプリかはこれで足りる。
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "(分からない)".to_owned())
}

/// いまの時刻。日付は要らない。一度の確認の中で前後が分かればよい。
fn now() -> String {
    // SAFETY: 値を返すだけの呼び出しで、引数も持たない。
    let time = unsafe { GetLocalTime() };
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        time.wHour, time.wMinute, time.wSecond, time.wMilliseconds
    )
}

/// 一行書く。失敗しても何もしない。記録できないことで入力を止めない。
pub fn write(message: &str) {
    let Some(path) = destination() else {
        return;
    };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(file, "[{} {}] {message}", now(), std::process::id());
}
