//! 頼みに答える係。
//!
//! **辞書を持っているのはここだけである。** TIP は引きたいものを頼み、
//! 返ってきたものを使う。ユーザー辞書の書き手も一つに絞られ、学習が
//! 競り合って壊れることがなくなる (ADR-0016)。
//!
//! ここには Windows が出てこない。パイプの向こうから来た一行をどう
//! 解釈するか、それだけを担う。**運び方と、答え方を分けてある。**

use crystalskk_core::dict::{CandidateSource, Query};
use crystalskk_dict::{MemoryDict, UserDict};
use crystalskk_ipc::{Request, Response};

/// 頼みを聞き終えたあと、どうするか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// 待ち続ける。
    Listen,
    /// 畳む。
    Stop,
}

/// 辞書を持ち、頼みに答える係。
#[derive(Debug)]
pub struct Service {
    /// 静的辞書。読むだけ。
    system: MemoryDict,
    /// ユーザー辞書。**この機械で唯一の書き手がここにいる。**
    user: UserDict,
}

impl Service {
    pub fn new(system: MemoryDict, user: UserDict) -> Self {
        Self { system, user }
    }

    /// 一つの頼みに答える。
    pub fn handle(&mut self, request: Request) -> (Response, Next) {
        match request {
            Request::Search(query) => (Response::Ok(self.search(&query)), Next::Listen),
            Request::Learn { query, word } => {
                self.user.learn(&query, &word);
                (Response::Ok(Vec::new()), Next::Listen)
            }
            Request::Register { query, word } => {
                self.user.learn(&query, &word);
                // 登録はその場で書き出す。**新しく覚えた語を落とすと、
                // 利用者の手間がそのまま失われる。**
                (self.save(), Next::Listen)
            }
            Request::Save => (self.save(), Next::Listen),
            // 答えてから畳む。頼んだ側は「聞き届けた」ことを知れる。
            Request::Exit => (self.save(), Next::Stop),
        }
    }

    /// ユーザー辞書を先に、静的辞書を後に引く。
    ///
    /// 順番がそのまま候補の並びになる。**一度選んだ語が先に出る**のは
    /// この順番による。
    fn search(&self, query: &Query) -> Vec<crystalskk_core::dict::Candidate> {
        let mut candidates = self.user.dict().lookup(query);
        for candidate in self.system.lookup(query) {
            if !candidates.iter().any(|seen| seen.word == candidate.word) {
                candidates.push(candidate);
            }
        }
        candidates
    }

    /// 書き出す。変更が無ければ [`UserDict::save`] が何もしない。
    fn save(&mut self) -> Response {
        match self.user.save() {
            Ok(()) => Response::Ok(Vec::new()),
            Err(e) => Response::Error(format!("ユーザー辞書を書けません: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crystalskk_core::dict::Candidate;

    /// 試験用。ユーザー辞書は一時の場所に置き、書き出しても実害が出ない
    /// ようにする。
    fn service_with(entries: &str) -> Service {
        let (system, _) = MemoryDict::parse(entries);
        let path = std::env::temp_dir().join(format!(
            "crystalskk-server-test-{}-{:?}.dict",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        Service::new(system, UserDict::new(path))
    }

    fn service() -> Service {
        service_with("かんじ /漢字/感じ/\n")
    }

    fn words(response: &Response) -> Vec<String> {
        match response {
            Response::Ok(candidates) => candidates.iter().map(|c| c.word.clone()).collect(),
            Response::Error(reason) => panic!("失敗した: {reason}"),
        }
    }

    #[test]
    fn a_search_returns_what_the_dictionary_holds() {
        let mut service = service();
        let (response, next) = service.handle(Request::Search(Query::okuri_nashi("かんじ")));
        assert_eq!(words(&response), vec!["漢字", "感じ"]);
        assert_eq!(next, Next::Listen);
    }

    #[test]
    fn an_unknown_heading_is_not_an_error() {
        // 辞書に無いことと、引けなかったことは別である。**取り違えると、
        // サーバが落ちていても「その語は無い」に見えてしまう。**
        let mut service = service();
        let (response, _) = service.handle(Request::Search(Query::okuri_nashi("ない")));
        assert_eq!(response, Response::Ok(Vec::new()));
    }

    #[test]
    fn what_was_learned_comes_first_next_time() {
        let mut service = service();
        let query = Query::okuri_nashi("かんじ");
        service.handle(Request::Learn {
            query: query.clone(),
            word: "感じ".to_owned(),
        });

        let (response, _) = service.handle(Request::Search(query));
        assert_eq!(words(&response), vec!["感じ", "漢字"], "選んだ語が先に出る");
    }

    #[test]
    fn a_registered_word_joins_the_candidates() {
        let mut service = service();
        let query = Query::okuri_nashi("あたらしい");
        service.handle(Request::Register {
            query: query.clone(),
            word: "新しい".to_owned(),
        });

        let (response, _) = service.handle(Request::Search(query));
        assert_eq!(words(&response), vec!["新しい"]);
    }

    #[test]
    fn the_same_word_is_not_listed_twice() {
        let mut service = service();
        let query = Query::okuri_nashi("かんじ");
        service.handle(Request::Learn {
            query: query.clone(),
            word: "漢字".to_owned(),
        });

        let (response, _) = service.handle(Request::Search(query));
        assert_eq!(words(&response), vec!["漢字", "感じ"], "重ねて出さない");
    }

    #[test]
    fn stopping_is_answered_before_it_happens() {
        let mut service = service();
        let (response, next) = service.handle(Request::Exit);
        assert_eq!(response, Response::Ok(Vec::new()), "先に答える");
        assert_eq!(next, Next::Stop);
    }

    #[test]
    fn annotations_survive_the_lookup() {
        let mut service = service_with("はし /橋;bridge/\n");
        let (response, _) = service.handle(Request::Search(Query::okuri_nashi("はし")));
        assert_eq!(
            response,
            Response::Ok(vec![Candidate::with_annotation("橋", "bridge")])
        );
    }
}
