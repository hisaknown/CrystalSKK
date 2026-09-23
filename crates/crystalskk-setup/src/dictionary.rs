//! 辞書の取得と設置。
//!
//! 導入とは別の命令にしてある。辞書は利用者ごとの場所へ置くので管理者
//! 権限が要らず、入れ替えも導入とは別の頻度で起きるため。

use std::process::ExitCode;

use crystalskk_server::paths;

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

    // 取り下げは**置いたあとで**。置き換えられたファイルは新しく作られる
    // ので、先に外しても意味がない。
    revoke_access();

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

/// かつて辞書に与えた、隔離された入れ物からの読みを取り下げる。
///
/// **辞書サーバができて要らなくなった** (ADR-0016)。読むのはサーバだけで、
/// サーバは隔離された入れ物の外にいる。
///
/// 外せなくても入力はできるので、失敗しても先へ進む。
pub fn revoke_access() {
    let Ok(directory) = paths::data_dir() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&directory) {
        eprintln!("crystalskk-setup: 置き場所を作れません: {e}");
        return;
    }
    let files = [paths::system_dictionary(), paths::user_dictionary()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if let Err(e) = access::revoke_app_containers(&directory, &files) {
        eprintln!("crystalskk-setup: {e}");
    }
}
