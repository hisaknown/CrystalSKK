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
//! # まず自分で起こす
//!
//! 繋がらなかったとき、利用者に「サーバを起こしてください」と言っても
//! 始まらない。**どうすればいいか分からないものを見せるのは、知らせでは
//! なく行き止まりである。**
//!
//! だから、引く前に居るかを見て、居なければ起こす。CorvusSKK も引くたびに
//! `_StartManager()` を呼んでいる。たいていの場面ではこれで直り、利用者は
//! 何も見ない。
//!
//! 起こせない場面もある。隔離された入れ物の中からはプロセスを作れない。
//! そのときだけ、繋がらないことを**隠さずに言う**。「辞書に無い」と
//! 「引けなかった」が混ざると、利用者からは「候補がおかしい」としか
//! 見えなくなる。

use std::cell::RefCell;
use std::rc::Rc;

use crystalskk_core::Engine;
use crystalskk_core::dict::{Candidate, CandidateSource, Query};
use crystalskk_ipc::{Request, Response};
use crystalskk_server::client;

use crate::{launch, log};

/// 繋がらなかったときに出す知らせ。
///
/// **候補が空なのは「辞書に無い」からだ、と思わせない。** 引けなかったのか
/// 辞書に無かったのかが混ざると、利用者には「候補がおかしい」としか見えない。
///
/// 起こし直しても駄目だったときにしか出ない。**出るからには、利用者に
/// できることを書く。**
pub const UNREACHABLE_NOTICE: &str = "辞書に繋がりません (別のアプリを開くと直ることがあります)";

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
        let request = Request::Search(query.clone());
        match client::ask(&request) {
            Ok(Response::Ok(candidates)) => {
                self.unreachable.set(false);
                return candidates;
            }
            Ok(Response::Error(reason)) => {
                // 答えは返っている。居ないわけではないので、起こしても
                // 意味がない。
                log::error(&format!("辞書サーバが断りました: {reason}"));
                self.unreachable.set(true);
                return Vec::new();
            }
            Err(e) => log::write(&format!("辞書サーバが居ません ({e})。起こします")),
        }

        // 居なかった。起こして、もう一度だけ尋ねる。
        if !launch::server() {
            self.unreachable.set(true);
            return Vec::new();
        }
        match client::ask(&request) {
            Ok(Response::Ok(candidates)) => {
                log::write("辞書サーバが起きました");
                self.unreachable.set(false);
                candidates
            }
            _ => {
                log::error("起こしても辞書サーバに繋がりません");
                self.unreachable.set(true);
                Vec::new()
            }
        }
    }

    /// いま引ける状態か。
    ///
    /// 直近の引き方が届いていれば引ける。**届かなかったことをエンジンへ
    /// 伝えるのがここの役目**で、伝わらないと「候補が無い」と見分けが
    /// つかず、辞書登録が始まってしまう。
    fn available(&self) -> bool {
        !self.unreachable.get()
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

    fn available(&self) -> bool {
        self.0.available()
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
