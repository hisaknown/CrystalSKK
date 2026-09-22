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
//!
//! # 包装されたアプリでは、環境変数も置き場所も当てにならない
//!
//! ストアの仕組みで包装されたアプリ (MSIX) は隔離された入れ物の中で動く。
//! そこでは二つのことが崩れる。
//!
//! - **環境変数が届かない。** 包装されたアプリは起動の道筋が違うので、
//!   `setx` で設定した値を受け取るとは限らない
//! - **書き込み先がすり替わる。** `%LOCALAPPDATA%` は入れ物ごとの場所へ
//!   向けられ、記録は
//!   `%LOCALAPPDATA%\Packages\<包装の名前>\AC\CrystalSKK\tip.log`
//!   に落ちる
//!
//! 前者は致命的で、**記録が無いことが「読み込まれていない」証しにならなく
//! なる**。診断の道具としては使い物にならない。
//!
//! そこで、環境変数に加えて**目印のファイル**でも記録を始められるようにする。
//! 置き場所は DLL の隣で、隔離された入れ物からも読める。

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
            switched_on().then_some(())?;
            let base = std::env::var_os("LOCALAPPDATA")?;
            let directory = PathBuf::from(base).join("CrystalSKK");
            std::fs::create_dir_all(&directory).ok()?;
            let path = directory.join("tip.log");
            announce(&path);
            Some(path)
        })
        .as_ref()
}

/// 記録するよう頼まれているか。
///
/// 環境変数と目印のファイルのどちらでもよい。包装されたアプリには環境変数が
/// 届かないことがあるので、ファイルという逃げ道を用意している。
fn switched_on() -> bool {
    std::env::var_os("CRYSTALSKK_LOG").is_some() || marker().is_some_and(|path| path.exists())
}

/// 目印のファイルの場所。DLL と同じところに置く。
///
/// 隔離された入れ物の中のアプリからも読めるよう、導入先に置くのが肝心で、
/// 利用者ごとの場所ではいけない。
fn marker() -> Option<PathBuf> {
    let module = crate::registry::module_path(crate::module()).ok()?;
    let directory = PathBuf::from(module).parent()?.to_path_buf();
    Some(directory.join(MARKER_NAME))
}

/// 目印のファイルの名前。中身は見ない。あるかどうかだけを見る。
pub const MARKER_NAME: &str = "log.on";

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
