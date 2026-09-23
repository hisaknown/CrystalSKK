//! 辞書を持つプロセス。
//!
//! 起動したら辞書を読み、パイプを開いて待つ。終われと言われるまで待ち
//! 続ける。

use std::process::ExitCode;

use crystalskk_dict::{MemoryDict, UserDict, encoding};
use crystalskk_ipc::{Request, Response};
use crystalskk_server::service::{Next, Service};
use crystalskk_server::{client, names, paths, pipe};

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => serve(),
        Some("stop") => stop(),
        Some("status") => status(),
        Some(other) => {
            eprintln!("crystalskk-server: 知らない指定です: {other}");
            eprintln!("使い方: crystalskk-server [serve|stop|status]");
            ExitCode::FAILURE
        }
    }
}

/// 動いているサーバに終わってくれと頼む。
///
/// **落とすのではなく頼む。** サーバはユーザー辞書の唯一の書き手なので、
/// 強制終了は書きかけの学習を捨てることになる。
fn stop() -> ExitCode {
    match client::ask(&Request::Exit) {
        Ok(Response::Ok(_)) => {
            println!("終了を頼みました。");
            ExitCode::SUCCESS
        }
        Ok(Response::Error(reason)) => {
            eprintln!("crystalskk-server: 断られました: {reason}");
            ExitCode::FAILURE
        }
        Err(_) => {
            println!("動いていません。");
            ExitCode::SUCCESS
        }
    }
}

/// 動いているかどうかを見る。
fn status() -> ExitCode {
    if client::is_running() {
        println!("動いています: {}", names::pipe());
    } else {
        println!("動いていません。");
    }
    ExitCode::SUCCESS
}

/// 辞書を読み、頼みを受け続ける。
fn serve() -> ExitCode {
    let Some(_only_one) = Singleton::claim(&names::mutex()) else {
        // すでに居る。二つ立てるとユーザー辞書の書き手が二つになる。
        eprintln!("crystalskk-server: すでに動いています");
        return ExitCode::SUCCESS;
    };

    let mut service = match load() {
        Ok(service) => service,
        Err(e) => {
            eprintln!("crystalskk-server: {e}");
            return ExitCode::FAILURE;
        }
    };

    let listener = match pipe::Listener::open(&names::pipe()) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("crystalskk-server: 待ち合わせ場所を開けません: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("crystalskk-server: {} で待っています", names::pipe());

    loop {
        let served = listener.serve_one(|line| {
            let Some(request) = Request::decode(line) else {
                // 読めない頼みは断る。**推し量って別の頼みとして扱わない。**
                return (
                    Response::Error("読めません".to_owned()).encode(),
                    Next::Listen,
                );
            };
            let (response, next) = service.handle(request);
            (response.encode(), next)
        });

        match served {
            Ok(Some(Next::Stop)) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("crystalskk-server: 待ち受けに失敗しました: {e}");
                break;
            }
        }
    }

    eprintln!("crystalskk-server: 終わります");
    ExitCode::SUCCESS
}

/// 辞書を読む。
fn load() -> std::io::Result<Service> {
    let system = match paths::system_dictionary() {
        Ok(path) => match std::fs::read(&path) {
            Ok(bytes) => {
                let decoded = encoding::decode(&bytes);
                let (dict, report) = MemoryDict::parse(&decoded.text);
                eprintln!(
                    "crystalskk-server: 辞書を読みました: {} 件 ({})",
                    report.entries, decoded.encoding
                );
                dict
            }
            Err(e) => {
                // 辞書が無くても待つ。**あとから置かれることもある**し、
                // ユーザー辞書だけでも引ける。
                eprintln!("crystalskk-server: 辞書がありません ({e})");
                MemoryDict::new()
            }
        },
        Err(e) => return Err(e),
    };

    let path = paths::user_dictionary()?;
    let user = match UserDict::load(&path) {
        Ok((dict, report)) => {
            eprintln!(
                "crystalskk-server: ユーザー辞書を読みました: {} 件",
                report.entries
            );
            dict
        }
        Err(e) => {
            eprintln!("crystalskk-server: ユーザー辞書を読めません ({e})");
            UserDict::new(path)
        }
    };

    Ok(Service::new(system, user))
}

/// 一つだけであることを示す印。
///
/// 持っている間だけサーバが名乗れる。落ちれば Windows が取り上げるので、
/// **次の起動が引っかかったままになることはない。**
struct Singleton(windows::Win32::Foundation::HANDLE);

impl Singleton {
    fn claim(name: &str) -> Option<Self> {
        let wide = windows::core::HSTRING::from(name);
        // SAFETY: 名前はこの関数で用意したもの。持ち手は `Drop` で閉じる。
        unsafe {
            let handle = windows::Win32::System::Threading::CreateMutexW(None, true, &wide).ok()?;
            // すでにあるなら、先客がいる。
            if windows::Win32::Foundation::GetLastError()
                == windows::Win32::Foundation::ERROR_ALREADY_EXISTS
            {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
                return None;
            }
            Some(Self(handle))
        }
    }
}

impl Drop for Singleton {
    fn drop(&mut self) {
        // SAFETY: 自分で作ったものを返す。
        unsafe {
            let _ = windows::Win32::System::Threading::ReleaseMutex(self.0);
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
