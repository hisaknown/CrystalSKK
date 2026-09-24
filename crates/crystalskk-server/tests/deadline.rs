//! パイプで待つ長さに上限があることを確かめる試験。
//!
//! 呼ぶのは入力先アプリの UI スレッドなので、**サーバが答えなければアプリ
//! ごと固まる**。実際にスタートメニューの検索欄が止まった。ここでは本物の
//! サーバではなく、試験ごとの名前で待ち合わせ場所を開いて確かめる。

#![cfg(windows)]

use std::io;
use std::time::{Duration, Instant};

use crystalskk_server::pipe;

/// 試験ごとに別の名前。本物のサーバとも、並んで走る試験とも重ならない。
fn unique_name(what: &str) -> String {
    format!(r"\\.\pipe\CrystalSKK.test.{what}.{}", std::process::id())
}

/// 待ち合わせ場所を別の糸で開き、一件だけ `answer` で答えさせる。
/// 開き終えてから返るので、すぐ繋ぎに行ってよい。
fn serve_once(
    name: &str,
    answer: impl FnOnce(&str) -> String + Send + 'static,
) -> std::thread::JoinHandle<()> {
    let name = name.to_owned();
    let (ready, opened) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let listener = pipe::Listener::open(&name).expect("開ける");
        ready.send(()).expect("知らせる");
        let _ = listener.serve_one(|line| (answer(line), ()));
    });
    opened.recv().expect("開き終わる");
    server
}

#[test]
fn a_missing_server_is_told_apart_as_not_found() {
    let error = pipe::ask(&unique_name("missing"), "q", 500).expect_err("居ない");
    assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error}");
}

#[test]
fn a_server_that_never_answers_does_not_hold_the_caller() {
    let name = unique_name("silent");
    // 受け取ったまま、期限よりずっと長く黙る。
    let server = serve_once(&name, |_| {
        std::thread::sleep(Duration::from_secs(3));
        String::new()
    });

    let started = Instant::now();
    let error = pipe::ask(&name, "q", 300).expect_err("答えは来ない");
    let waited = started.elapsed();

    assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
    assert!(waited < Duration::from_millis(1000), "待ちすぎ: {waited:?}");
    server.join().expect("サーバの糸が終わる");
}

#[test]
fn a_prompt_answer_comes_through() {
    let name = unique_name("prompt");
    let server = serve_once(&name, |line| format!("re:{line}"));

    let answer = pipe::ask(&name, "q", 500).expect("答えが来る");
    assert_eq!(answer, "re:q");
    server.join().expect("サーバの糸が終わる");
}
