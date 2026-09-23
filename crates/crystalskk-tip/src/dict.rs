//! 辞書との繋がり。
//!
//! **TIP は辞書を持たない。** 持っているのは辞書サーバで、こちらは引きたい
//! ものを頼む (ADR-0016)。
//!
//! # 逃げ道は作らない
//!
//! 繋がらなかったときに自分で辞書を読む道は用意していない。読み手が二つに
//! なれば書き手も二つになり、**学習が壊れる**。めったに通らない道は腐り
//! もする。
//!
//! 代わりに、繋がらないことを**隠さない**。「辞書に無い」と「引けなかった」
//! が混ざると、利用者からは「候補がおかしい」としか見えなくなる。

use std::cell::RefCell;
use std::rc::Rc;

use crystalskk_core::Engine;
use crystalskk_core::dict::{Candidate, CandidateSource, Query};
use crystalskk_ipc::{Request, Response};
use crystalskk_server::client;

use crate::log;

/// 繋がらなかったときに出す知らせ。
///
/// **候補が空なのは「辞書に無い」からだ、と思わせない。** 引けなかったのか
/// 辞書に無かったのかが混ざると、利用者には「候補がおかしい」としか見えない。
pub const UNREACHABLE_NOTICE: &str = "辞書に繋がりません";

/// 辞書サーバを候補の出どころとして使う。
///
/// エンジンから見れば、ただの [`CandidateSource`] である。**パイプの
/// 向こうにいることをエンジンは知らない。** ADR-0001 で切っておいた継ぎ目が
/// そのまま使えた。
#[derive(Debug, Default)]
pub struct ServerSource {
    /// 直近の引き方でサーバに届かなかったか。
    unreachable: std::cell::Cell<bool>,
}

impl ServerSource {
    fn new() -> Self {
        Self::default()
    }

    /// 直近の引き方でサーバに届かなかったか。
    pub fn was_unreachable(&self) -> bool {
        self.unreachable.get()
    }
}

impl CandidateSource for ServerSource {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        match client::ask(&Request::Search(query.clone())) {
            Ok(Response::Ok(candidates)) => {
                self.unreachable.set(false);
                candidates
            }
            Ok(Response::Error(reason)) => {
                log::error(&format!("辞書サーバが断りました: {reason}"));
                self.unreachable.set(true);
                Vec::new()
            }
            Err(e) => {
                log::error(&format!("辞書サーバに繋がりません: {e}"));
                self.unreachable.set(true);
                Vec::new()
            }
        }
    }
}

/// 引き手を共有するための持ち手。
///
/// エンジンは候補の出どころを所有するが、「繋がったか」は TIP 側でも
/// 知りたい。持ち手を分け合う。
#[derive(Debug, Clone)]
pub struct SharedSource(Rc<ServerSource>);

impl SharedSource {
    /// 直近の引き方でサーバに届かなかったか。
    pub fn was_unreachable(&self) -> bool {
        self.0.was_unreachable()
    }
}

impl CandidateSource for SharedSource {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.0.lookup(query)
    }
}

/// 学習をサーバへ伝える係。
///
/// 書くのはサーバだけである。**こちらはファイルに触れない。**
#[derive(Debug, Default, Clone)]
pub struct Learning {
    /// 伝えそこねた学習。次の折に送り直す。
    pending: Rc<RefCell<Vec<Request>>>,
}

impl Learning {
    /// 選ばれた候補を覚えさせる。
    pub fn learn(&self, query: Query, word: String) {
        self.send(Request::Learn { query, word });
    }

    /// 新しい語を登録させる。
    pub fn register(&self, query: Query, word: String) {
        self.send(Request::Register { query, word });
    }

    /// 書き出させる。
    pub fn save(&self) {
        self.send(Request::Save);
    }

    /// 頼みを一つ送る。送れなければ溜めておく。
    ///
    /// **学習は落としたくないが、入力を止めてまで守るものでもない。**
    /// 次に送れたときに一緒に流す。
    fn send(&self, request: Request) {
        let mut pending = self.pending.borrow_mut();
        pending.push(request);

        let mut unsent = Vec::new();
        for request in pending.drain(..) {
            match client::ask(&request) {
                Ok(Response::Ok(_)) => {}
                Ok(Response::Error(reason)) => {
                    log::error(&format!("学習を断られました: {reason}"));
                }
                Err(_) => unsent.push(request),
            }
        }
        if !unsent.is_empty() {
            log::error(&format!("学習を {} 件ためています", unsent.len()));
        }
        *pending = unsent;
    }
}

/// エンジンと、学習を伝える係を用意する。
pub fn build() -> (Engine, SharedSource, Learning) {
    let source = SharedSource(Rc::new(ServerSource::new()));
    let engine = Engine::new(Box::new(source.clone()));
    (engine, source, Learning::default())
}
