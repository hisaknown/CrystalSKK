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
use crystalskk_core::dict::{Candidate, CandidateSource, Context, Query};
use crystalskk_ipc::{Request, Response};
use crystalskk_server::client;

use crate::{launch, log};

/// 繋がらなかったときに出す知らせ。
///
/// **候補が空なのは「辞書に無い」からだ、と思わせない。** 引けなかったのか
/// 辞書に無かったのかが混ざると、利用者には「候補がおかしい」としか見えない。
///
/// 起こし直しても駄目だったときにしか出ない。**出るからには、利用者に
/// できることを書く。** 次に変換したとき、設定なら数秒後の打鍵で、
/// こちらから繋ぎ直す。利用者は少し待てばよい。
pub const UNREACHABLE_NOTICE: &str = "辞書に繋がりません (少し待つと繋がり直します)";

/// 辞書サーバを候補の出どころとして使う。
///
/// エンジンから見れば、ただの [`CandidateSource`] である。**パイプの
/// 向こうにいることをエンジンは知らない。** ADR-0001 で切っておいた継ぎ目が
/// そのまま使えた。
#[derive(Debug, Default)]
pub struct ServerSource {
    /// 直近の引き方で引けなかったなら、その訳。**利用者に見せる文。**
    ///
    /// サーバに届かなかったときと、届いたが引けなかったとき (辞書を取得
    /// している最中など) がある。どちらも「辞書に無い」とは違う。
    trouble: std::cell::RefCell<Option<String>>,
    /// 変換のときにサーバへ見せる、カーソルの前と後の文字数。サーバが
    /// 候補を並べ替えない設定なら `None` で、前後の文章は送らない。
    reach: std::cell::Cell<Option<(usize, usize)>>,
}

impl ServerSource {
    fn new() -> Self {
        Self::default()
    }

    /// 直近の引き方で引けなかったか。
    pub fn was_unreachable(&self) -> bool {
        self.trouble.borrow().is_some()
    }

    /// 直近の引き方で引けなかった訳。
    pub fn notice(&self) -> Option<String> {
        self.trouble.borrow().clone()
    }

    /// 答えを候補にする。引けなかったなら訳を覚えて、空を返す。
    fn answer(&self, response: Response) -> Vec<Candidate> {
        let (candidates, trouble) = match response {
            Response::Ok(candidates) => (candidates, None),
            // 答えは返っている。居ないわけではないので、起こしても意味が
            // ない。**言われたことをそのまま伝える** (「辞書を取得して
            // います」など)。
            Response::Error(reason) => {
                log::error(&format!("辞書サーバが断りました: {reason}"));
                (Vec::new(), Some(reason))
            }
            Response::Settings { .. } | Response::Done(_) => {
                log::error("検索に候補ではないものが返りました");
                (Vec::new(), Some(UNREACHABLE_NOTICE.to_owned()))
            }
        };
        *self.trouble.borrow_mut() = trouble;
        candidates
    }

    /// 候補を頼む。居なければ起こして、起きるのを少しだけ待つ。
    fn ask_for_candidates(&self, request: &Request) -> Vec<Candidate> {
        match ask_waking(request) {
            Ok(response) => self.answer(response),
            Err(_) => {
                *self.trouble.borrow_mut() = Some(UNREACHABLE_NOTICE.to_owned());
                Vec::new()
            }
        }
    }
}

impl CandidateSource for ServerSource {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.ask_for_candidates(&Request::Search(query.clone()))
    }

    /// 変換のために引く。サーバが並べ替える設定なら、前後の文章を添える。
    ///
    /// **並べ替えないなら前後の文章は送らない。** 使われない文章を
    /// パイプに流すことはない。
    fn lookup_for_conversion(&self, query: &Query, context: &Context) -> Vec<Candidate> {
        let Some((before, after)) = self.reach.get() else {
            return self.lookup(query);
        };
        self.ask_for_candidates(&Request::Convert {
            query: query.clone(),
            before: context.text_before(before),
            after: context.text_after(after),
        })
    }

    /// 前方一致する見出しを返す。補完に使う。
    ///
    /// **出なくても騒がない。** 補完は出れば助かるもので、出なければ出ないだけ
    /// である。ここで繋がらないことを言い立てると、打鍵のたびに知らせが
    /// 出ることになる。
    fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        let request = Request::Complete {
            prefix: prefix.to_owned(),
            limit,
        };
        match client::ask(&request) {
            Ok(Response::Ok(found)) => found.into_iter().map(|c| c.word).collect(),
            _ => Vec::new(),
        }
    }

    /// いま引ける状態か。
    ///
    /// 直近の引き方が届いていれば引ける。**届かなかったことをエンジンへ
    /// 伝えるのがここの役目**で、伝わらないと「候補が無い」と見分けが
    /// つかず、辞書登録が始まってしまう。
    fn available(&self) -> bool {
        self.trouble.borrow().is_none()
    }
}

/// 引き手を共有するための持ち手。
///
/// エンジンは候補の出どころを所有するが、「繋がったか」は TIP 側でも
/// 知りたい。持ち手を分け合う。
#[derive(Debug, Clone)]
pub struct SharedSource(Rc<ServerSource>);

impl SharedSource {
    /// 直近の引き方で引けなかった訳。引けていれば `None`。
    pub fn notice(&self) -> Option<String> {
        self.0.notice()
    }

    /// 並べ替えの設定を受け取る。変換のときに前後の文章を何文字送るかが
    /// 決まる。
    pub fn set_ranking(&self, ranker: &crystalskk_settings::Ranker) {
        self.0
            .reach
            .set(ranker.enabled.then_some((ranker.before, ranker.after)));
    }

    /// 前後の文章を読むべきか。サーバが並べ替えない設定なら読まない。
    pub fn wants_surroundings(&self) -> bool {
        self.0.reach.get().is_some()
    }
}

impl CandidateSource for SharedSource {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.0.lookup(query)
    }

    fn lookup_for_conversion(&self, query: &Query, context: &Context) -> Vec<Candidate> {
        self.0.lookup_for_conversion(query, context)
    }

    fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        self.0.complete(prefix, limit)
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
                Ok(Response::Ok(_) | Response::Settings { .. } | Response::Done(_)) => {}
                Ok(Response::Error(reason)) => {
                    log::error(&format!("学習を断られました: {reason}"));
                }
                // 送ったが答えが来なかった。**届いてはいるかもしれない**
                // ので送り直さない。二重に覚えさせるより、一度落とすほうが
                // 害が小さい。
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    log::error(&format!("学習の返事が来ません。送り直しません: {e}"));
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

/// 設定を尋ねる。
///
/// **設定ファイルを読むのはサーバである。** TIP は隔離された入れ物の中
/// からではファイルを読めないし、足りない項目を書き足す書き手は一人に
/// 絞りたい。こちらは全文を受け取り、同じ crate で読む。
///
/// 居なければ起こして、もう一度だけ尋ねる。辞書を引くときと同じである。
///
/// 返す誤りは**そのまま利用者に見せる文**になっている。
pub fn fetch_settings() -> Result<crystalskk_settings::Settings, String> {
    match ask_server(&Request::Settings)? {
        Response::Settings { config, romaji } => crystalskk_settings::parse(&config, &romaji)
            .map_err(|e| format!("設定を読めません: {e}")),
        Response::Error(reason) => Err(format!("設定を読めません: {reason}")),
        Response::Ok(_) | Response::Done(_) => {
            Err("設定を読めません: 辞書サーバの答えが噛み合いません".to_owned())
        }
    }
}

/// サーバに頼み、したことを知らせる文を受け取る。品書きから使う。
pub fn ask_to_do(request: &Request) -> Result<String, String> {
    match ask_server(request)? {
        Response::Done(told) => Ok(told),
        Response::Error(reason) => Err(reason),
        Response::Ok(_) | Response::Settings { .. } => {
            Err("辞書サーバの答えが噛み合いません".to_owned())
        }
    }
}

/// サーバに頼む。繋がらなければ**そのまま利用者に見せる文**を返す。
fn ask_server(request: &Request) -> Result<Response, String> {
    ask_waking(request).map_err(|_| UNREACHABLE_NOTICE.to_owned())
}

/// サーバに頼む。居なければ起こして、起きるのを少しだけ待つ。
///
/// 起こすのは**居ない**ときだけである。居るのに答えない (忙しい、壊れて
/// いる) なら、起こしても尋ね直しても直らず、アプリを止める時間が
/// 重なるだけになる。
fn ask_waking(request: &Request) -> std::io::Result<Response> {
    let absent = match client::ask(request) {
        Ok(response) => return Ok(response),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => e,
        Err(e) => {
            log::error(&format!("辞書サーバに頼めません: {e}"));
            return Err(e);
        }
    };
    log::write(&format!("辞書サーバが居ません ({absent})。起こします"));
    if !launch::server() {
        return Err(absent);
    }
    match launch::wait_until_up(|| client::ask(request)) {
        Ok(response) => {
            log::write("辞書サーバが起きました");
            Ok(response)
        }
        Err(e) => {
            log::error(&format!("起こしても辞書サーバに繋がりません: {e}"));
            Err(e)
        }
    }
}

/// エンジンと、学習を伝える係を用意する。
pub fn build() -> (Engine, SharedSource, Learning) {
    let source = SharedSource(Rc::new(ServerSource::new()));
    let engine = Engine::new(Box::new(source.clone()));
    (engine, source, Learning::default())
}
