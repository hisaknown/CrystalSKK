//! 実際に立てたサーバと話してみる試験。
//!
//! 語彙と処理はそれぞれ単体で試験してあるが、**パイプの上で繋がるか**は
//! 動かしてみないと分からない。名前の付け方、許可の書き方、読み書きの型 —
//! どれも間違えていれば、ここで落ちる。
//!
//! すでにサーバが動いていればそれと話す。動いていなければ何もしない。
//! **試験のために勝手にプロセスを立てたり落としたりしない。**

use crystalskk_core::dict::Query;
use crystalskk_ipc::{Request, Response};
use crystalskk_server::client;

/// 動いていなければ飛ばす。
fn running() -> bool {
    let answer = client::ask(&Request::Search(Query::okuri_nashi("かんじ")));
    answer.is_ok()
}

#[test]
fn a_running_server_answers_a_search() {
    if !running() {
        eprintln!("サーバが動いていないので飛ばす");
        return;
    }

    let answer = client::ask(&Request::Search(Query::okuri_nashi("かんじ")))
        .expect("繋がっているなら答えが返る");
    let Response::Ok(candidates) = answer else {
        panic!("引けるはず");
    };
    assert!(
        candidates.iter().any(|c| c.word == "漢字"),
        "L 辞書があれば「漢字」が出る: {candidates:?}"
    );
}

#[test]
fn an_unknown_heading_comes_back_empty_not_broken() {
    if !running() {
        return;
    }

    let answer = client::ask(&Request::Search(Query::okuri_nashi(
        "このみだしごはじしょにない",
    )))
    .expect("繋がっているなら答えが返る");
    assert_eq!(answer, Response::Ok(Vec::new()));
}

#[test]
fn an_okuri_ari_heading_keeps_its_okuri() {
    if !running() {
        return;
    }

    let query = Query::okuri_ari("おく", 'r', "り");
    let answer = client::ask(&Request::Search(query)).expect("答えが返る");
    let Response::Ok(candidates) = answer else {
        panic!("引けるはず");
    };
    assert!(
        candidates.iter().any(|c| c.word == "送"),
        "送りありも引ける: {candidates:?}"
    );
}
