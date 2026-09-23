//! 設定ファイルの用意。
//!
//! 導入のたびに通る。初めてなら雛形から作り、新しい版で項目が増えていれば
//! 書き足す (ADR-0020)。**何を書き足したかは、ここで利用者に見せる。**
//! 導入のあとは辞書サーバが同じことをするが、サーバには知らせる相手が
//! いない。
//!
//! # 本人として動いているうちにやる
//!
//! 設定ファイルは利用者ごとの場所にある。昇格した側から見える
//! `%LOCALAPPDATA%` は、別の管理者のものかもしれない。
//!
//! # 古いサーバとかち合っても壊れない
//!
//! この時点ではまだ古い辞書サーバが動いていることがある。どちらも「読んだ
//! 全文に足りない項目を足して、全文を置き換える」だけなので、同じ項目が
//! 二度書かれることはない。古いほうの書き込みが後に来て新しい項目が
//! 消えても、新しいサーバが次に読んだときに書き足す。

use crystalskk_server::paths;

/// 設定ファイルを用意し、したことを伝える。
///
/// 失敗しても導入は続ける。**設定が読めなければ入力は動かない**が、
/// 登録まで止めると、直したあとにもう一度導入し直すことになる。
pub fn prepare() {
    let path = match paths::settings() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("crystalskk-setup: 設定ファイルの置き場所が分かりません: {e}");
            return;
        }
    };

    match crystalskk_settings::load(&path) {
        Ok(loaded) => {
            if loaded.created {
                println!("設定ファイルを作りました: {}", path.display());
            } else if !loaded.added.is_empty() {
                println!("設定ファイルに項目を書き足しました: {}", path.display());
                for key in &loaded.added {
                    println!("  {key}");
                }
            }
            if !loaded.unknown.is_empty() {
                println!("設定ファイルに知らない項目があります (消してはいません):");
                for key in &loaded.unknown {
                    println!("  {key}");
                }
            }
            // ローマ字テーブルは無いときに作るだけで、あれば触らない
            // (ADR-0021)。作ったときだけ言う。
            if loaded.romaji_created {
                println!(
                    "ローマ字テーブルを作りました: {}",
                    loaded.romaji_path.display()
                );
            }
            // かつての L 辞書は、もう読まない。
            crate::dictionary::remove_legacy();
        }
        Err(e) => {
            eprintln!("crystalskk-setup: {e}");
            eprintln!("  直すまで、日本語の入力はできません: {}", path.display());
        }
    }
}
