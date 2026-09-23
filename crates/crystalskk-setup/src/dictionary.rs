//! 辞書の取得と設置。
//!
//! 導入とは別の命令にしてある。辞書は利用者ごとの場所へ置くので管理者
//! 権限が要らず、入れ替えも導入とは別の頻度で起きるため。

use std::process::ExitCode;

use crystalskk_tip::paths;

use crate::access;

/// 辞書を取得して、CrystalSKK が読む場所へ置く。
pub fn fetch() -> ExitCode {
    let path = match paths::system_dictionary() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("crystalskk-setup: 辞書の置き場所が分かりません: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("取得元: {}", crystalskk_fetch::SKK_JISYO_L);
    println!("置き場: {}", path.display());

    let outcome = crystalskk_fetch::install(crystalskk_fetch::SKK_JISYO_L, &path, None);

    // 許可は**置いたあとで**与える。先に与えても、置き換えられた
    // ファイルは新しく作られるので引き継がれない。
    grant_access();

    match outcome {
        Ok(Some(report)) => {
            println!();
            println!("見出し:     {} 件", report.entries);
            println!("取得元符号: {}", report.source_encoding);
            if report.skipped > 0 {
                println!("読み飛ばし: {} 行", report.skipped);
            }
            if report.merged > 0 {
                println!("併合:       {} 行", report.merged);
            }
            println!();
            println!("次に起動したアプリから使われます。");
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("変化がないので、そのままにしました。");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("crystalskk-setup: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 辞書の置き場所に、隔離された入れ物からの読みを許す。
///
/// **ストアアプリやスタートメニューの検索欄は、この許可が無いと辞書を
/// 読めない。** 与えられなくても普通のアプリでは使えるので、失敗しても
/// 取得そのものは続ける。
pub fn grant_access() {
    let Ok(directory) = paths::data_dir() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&directory) {
        eprintln!("crystalskk-setup: 置き場所を作れません: {e}");
        return;
    }
    // 読ませるのは辞書だけ。診断の記録は同じ場所にあるが、挙げない。
    let readable = [paths::system_dictionary(), paths::user_dictionary()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    match access::allow_app_containers(&directory, &readable) {
        Ok(()) => println!("隔離されたアプリからも読めるようにしました。"),
        Err(e) => eprintln!("crystalskk-setup: {e}"),
    }
}
