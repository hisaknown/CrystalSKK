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

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

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
            Some(directory.join("tip.log"))
        })
        .as_ref()
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
    let _ = writeln!(file, "[{}] {message}", std::process::id());
}
