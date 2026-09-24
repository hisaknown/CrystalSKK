//! サーバに話しかける側。
//!
//! TIP と導入ツールが使う。**繋がらないことを隠さない**のが肝心で、
//! 引けなかったのか辞書に無かったのかが混ざると、利用者には「候補が
//! おかしい」としか見えなくなる。

use std::io;

use crystalskk_core::dict::{Candidate, CandidateSource, Query};
use crystalskk_ipc::{Request, Response};

use crate::{names, pipe};

/// 一つの頼みに待つ長さ。繋ぐところから答えを読み終えるまでを合わせて数える
/// (ADR-0032)。
///
/// 呼ぶのは入力先アプリの UI スレッドなので、**これがそのままアプリの
/// 固まる長さの上限になる**。狙いは異常なときにいつまでも止まらないこと
/// で、普段の答えは数ミリ秒で返る。並べ替えのある変換も、これに収まる。
const TIMEOUT_MS: u32 = 500;

/// サーバへ頼みを一つ送る。
///
/// 誤りの種類の意味は [`pipe::ask`] を見よ。
pub fn ask(request: &Request) -> io::Result<Response> {
    let line = pipe::ask(&names::pipe(), &request.encode(), TIMEOUT_MS)?;
    Response::decode(&line).ok_or_else(|| io::Error::other(format!("答えを読めません: {line:?}")))
}

/// サーバが待っているか。
pub fn is_running() -> bool {
    // 頼みを送らずに確かめる手もあるが、**答えられる状態か**までは分から
    // ない。一番軽い頼みを実際に投げるのが確かである。
    matches!(ask(&Request::Save), Ok(Response::Ok(_)))
}

/// 辞書サーバを候補の出どころとして使う。
///
/// エンジンから見れば、ただの [`CandidateSource`] である。**パイプの
/// 向こうにいることをエンジンは知らない** (ADR-0001 の継ぎ目)。
#[derive(Debug, Default)]
pub struct ServerSource {
    /// 直近の引き方が失敗したか。
    ///
    /// 「辞書に無い」と「引けなかった」を区別して伝えるために持つ。
    unreachable: std::cell::Cell<bool>,
}

impl ServerSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// 直近の引き方でサーバに届かなかったか。
    pub fn was_unreachable(&self) -> bool {
        self.unreachable.get()
    }
}

impl CandidateSource for ServerSource {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        match ask(&Request::Search(query.clone())) {
            Ok(Response::Ok(candidates)) => {
                self.unreachable.set(false);
                candidates
            }
            // 検索に設定が返るのは、話が噛み合っていないということ。
            Ok(Response::Error(_) | Response::Settings { .. } | Response::Done(_)) | Err(_) => {
                self.unreachable.set(true);
                Vec::new()
            }
        }
    }
}
