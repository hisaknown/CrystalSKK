//! CrystalSKK をこの環境に導入する。
//!
//! ```text
//! crystalskk-setup install [DLL]
//! crystalskk-setup uninstall [--purge]
//! crystalskk-setup status
//! ```
//!
//! 管理者権限は要らない。登録先は利用者ごとの領域である (ADR-0006)。
//!
//! これは配布用のインストーラではない。本物のインストーラを作るときは、
//! この処理をそのまま中身として使う。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crystalskk_tip::com::Apartment;

mod install;

fn main() -> ExitCode {
    // 入力方式の登録は COM 越しに行う。抜けるのは持ち手が面倒を見る。
    let _apartment = Apartment::enter();
    run()
}

fn run() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();

    match arguments.as_slice() {
        [] | ["-h"] | ["--help"] | ["help"] => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        ["status"] => report_status(),
        ["install", rest @ ..] => match rest {
            [] => do_install(None),
            [path] => do_install(Some(Path::new(path))),
            _ => fail("install が受け取れるのは DLL 一つまでです"),
        },
        ["uninstall", rest @ ..] => match rest {
            [] => do_uninstall(false),
            ["--purge"] => do_uninstall(true),
            _ => fail("uninstall が受け取れるのは --purge だけです"),
        },
        [other, ..] => fail(&format!("知らない命令です: {other}")),
    }
}

fn do_install(source: Option<&Path>) -> ExitCode {
    let source = match source.map(PathBuf::from).or_else(default_source) {
        Some(path) => path,
        None => {
            return fail(
                "DLL が見つかりません。場所を渡すか、先に \
                 cargo build -p crystalskk-tip --release を実行してください",
            );
        }
    };

    match install::install(&source) {
        Ok(installed) => {
            let verb = if installed.replaced {
                "入れ替えました"
            } else {
                "導入しました"
            };
            println!("CrystalSKK を{verb}: {}", installed.dll.display());
            println!();
            println!("設定 → 時刻と言語 → 言語と地域 → 日本語 → 言語のオプション →");
            println!("キーボード に CrystalSKK が現れます。");
            println!("まだ入力はできません。有効化して選べるところまでです。");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.to_string()),
    }
}

fn do_uninstall(purge: bool) -> ExitCode {
    match install::uninstall(purge) {
        Ok(()) => {
            println!("CrystalSKK の登録を解除しました。");
            if purge {
                println!("写した DLL も削除しました。");
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.to_string()),
    }
}

fn report_status() -> ExitCode {
    match install::status() {
        install::Status::NotInstalled => {
            println!("導入されていません。");
        }
        install::Status::Installed { dll, present } => {
            println!("導入済み: {}", dll.display());
            if !present {
                println!();
                println!("登録されている場所に DLL がありません。");
                println!("uninstall してから install し直してください。");
            }
        }
    }
    ExitCode::SUCCESS
}

/// 場所を指定されなかったときに探す先。
///
/// 自分と同じ場所、その次にビルド成果物を見る。開発中は
/// `cargo run -p crystalskk-setup -- install` がそのまま通る。
fn default_source() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(directory) = exe.parent()
    {
        candidates.push(directory.join(install::DLL_NAME));
    }
    candidates.push(PathBuf::from("target/release").join(install::DLL_NAME));
    candidates.push(PathBuf::from("target/debug").join(install::DLL_NAME));
    candidates.into_iter().find(|path| path.is_file())
}

fn fail(message: &str) -> ExitCode {
    eprintln!("crystalskk-setup: {message}");
    ExitCode::FAILURE
}

const USAGE: &str = "\
crystalskk-setup - CrystalSKK をこの環境に導入する

使い方:
  crystalskk-setup install [DLL]      導入する (DLL 省略時はビルド成果物を探す)
  crystalskk-setup uninstall          登録を解除する
  crystalskk-setup uninstall --purge  写した DLL も削除する
  crystalskk-setup status             今の状態を表示する

管理者権限は要らない。登録は利用者ごとに行われる。
";
