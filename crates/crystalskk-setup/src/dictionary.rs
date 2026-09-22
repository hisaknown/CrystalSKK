//! 辞書の取得と設置。
//!
//! 導入とは別の命令にしてある。辞書は利用者ごとの場所へ置くので管理者
//! 権限が要らず、入れ替えも導入とは別の頻度で起きるため。

use std::process::ExitCode;

use crystalskk_dict::paths;

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

    match crystalskk_fetch::install(crystalskk_fetch::SKK_JISYO_L, &path, None) {
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
