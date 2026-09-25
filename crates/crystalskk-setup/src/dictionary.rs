//! 辞書の取得と設置。
//!
//! どの辞書を使うかは設定ファイルの `dictionaries.sources` で決める
//! (ADR-0022)。取得は辞書サーバが裏でやるので、ここは**いま**取り直したい
//! ときと、何が起きたかを見たいときのためにある。
//!
//! 導入とは別の命令にしてある。辞書は利用者ごとの場所へ置くので管理者
//! 権限が要らず、入れ替えも導入とは別の頻度で起きるため。

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crystalskk_fetch::{InstallReport, Progress};

use crystalskk_server::paths;
use crystalskk_settings::Source;

use crate::access;

/// 設定に並べた辞書を取り直す。
///
/// 辞書サーバも起動のたびに同じことをする (ADR-0022)。こちらは**いま**
/// 取り直したいとき、そして何が起きたかを見たいときに使う。サーバには
/// 知らせる相手がいない。
///
/// 変わっていなければ取り直さない (`ETag` で確かめる)。
pub fn fetch() -> ExitCode {
    let (settings_path, cache) = match (paths::settings(), paths::dictionary_cache()) {
        (Ok(settings), Ok(cache)) => (settings, cache),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("crystalskk-setup: 置き場所が分かりません: {e}");
            return ExitCode::FAILURE;
        }
    };
    let loaded = match crystalskk_settings::load(&settings_path) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("crystalskk-setup: {e}");
            return ExitCode::FAILURE;
        }
    };
    let directory = settings_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    let sources = &loaded.settings.dictionaries;
    if sources.is_empty() {
        println!("辞書が並べられていません (dictionaries.sources)。ユーザー辞書だけで使います。");
    }

    let mut failed = false;
    let mut fetched: Vec<String> = Vec::new();
    for source in sources {
        println!();
        if source.is_katakana() {
            // 作るのは辞書サーバが読み込むとき。ここでは元の辞書を揃える。
            println!(
                "カタカナ語の辞書: {} から作ります",
                source.base().as_written()
            );
        }
        // 同じ辞書は二度取らない。
        if fetched.contains(&source.base().as_written()) {
            continue;
        }
        fetched.push(source.base().as_written());
        match source.base() {
            Source::Url(url) => {
                let path = match crystalskk_fetch::cache_path(&cache, url) {
                    Ok(path) => path,
                    Err(e) => {
                        eprintln!("crystalskk-setup: {e}");
                        failed = true;
                        continue;
                    }
                };
                println!("取得元: {url}");
                println!("置き場: {}", path.display());
                match refresh_showing_progress(url, &path) {
                    Ok(Some(report)) => {
                        println!("見出し:     {} 件", report.entries);
                        println!("取得元符号: {}", report.source_encoding);
                        if report.skipped > 0 {
                            println!("読み飛ばし: {} 行", report.skipped);
                        }
                        if report.merged > 0 {
                            println!("併合:       {} 行", report.merged);
                        }
                    }
                    Ok(None) => println!("変化がないので、そのままにしました。"),
                    Err(e) => {
                        eprintln!("crystalskk-setup: {e}");
                        failed = true;
                    }
                }
            }
            Source::File(_) | Source::Katakana(_) => {
                let path = source
                    .resolve(&directory)
                    .expect("ファイルの在りかは必ず解ける");
                if path.is_file() {
                    println!("手元の辞書: {}", path.display());
                } else {
                    eprintln!("crystalskk-setup: 辞書が見つかりません: {}", path.display());
                    failed = true;
                }
            }
        }
    }

    remove_legacy();
    revoke_access();

    println!();
    println!("辞書サーバは、次に入力先を切り替えたときに読み直します。");
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// かつての置き場所に残っている L 辞書を片付ける。
///
/// 辞書を設定で選べるようにする前は、`%LOCALAPPDATA%\CrystalSKK\SKK-JISYO.L`
/// に一つだけ置いていた (ADR-0022)。いまは読まないので、4 MiB ほどの
/// 置き去りになる。**CrystalSKK が取ってきたもの**なので、黙って残さずに
/// 消す。
pub fn remove_legacy() {
    let Ok(path) = paths::system_dictionary() else {
        return;
    };
    if !path.is_file() {
        return;
    }
    match std::fs::remove_file(&path) {
        Ok(()) => println!("古い置き場所の辞書を片付けました: {}", path.display()),
        Err(e) => eprintln!(
            "crystalskk-setup: 古い置き場所の辞書を消せません ({}): {e}",
            path.display()
        ),
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

/// 取り直しながら、受け取った量を一秒ごとに書き出す。
///
/// 大きな辞書を遅い回線で取ると、黙ったままでは止まったのか分からない。
fn refresh_showing_progress(
    url: &str,
    path: &Path,
) -> Result<Option<InstallReport>, crystalskk_fetch::Error> {
    let progress = Progress::new();
    let started = Instant::now();
    std::thread::scope(|scope| {
        let (done, finished) = mpsc::channel::<()>();
        let shared = &progress;
        let worker = scope.spawn(move || {
            let result = crystalskk_fetch::refresh_with_progress(url, path, shared);
            let _ = done.send(());
            result
        });
        let mut shown = false;
        while let Err(mpsc::RecvTimeoutError::Timeout) =
            finished.recv_timeout(Duration::from_secs(1))
        {
            print!("\r取得中:     {}", describe(&progress, started.elapsed()));
            let _ = std::io::stdout().flush();
            shown = true;
        }
        if shown {
            println!();
        }
        worker.join().expect("取得のスレッドは倒れない")
    })
}

/// 進み具合を一行で。
fn describe(progress: &Progress, elapsed: Duration) -> String {
    let megabytes = |bytes: u64| format!("{:.1}", bytes as f64 / 1_000_000.0);
    let received = megabytes(progress.received());
    let size = match progress.total() {
        Some(total) => format!("{received} / {} MB", megabytes(total)),
        None => format!("{received} MB"),
    };
    format!("{size} ({} 秒)   ", elapsed.as_secs())
}
