//! 頼みに答える係。
//!
//! **辞書を持っているのはここだけである。** TIP は引きたいものを頼み、
//! 返ってきたものを使う。ユーザー辞書の書き手も一つに絞られ、学習が
//! 競り合って壊れることがなくなる (ADR-0016)。
//!
//! ここには Windows が出てこない。パイプの向こうから来た一行をどう
//! 解釈するか、それだけを担う。**運び方と、答え方を分けてある。**

use std::path::PathBuf;

use crystalskk_core::dict::{Candidate, CandidateSource, Query};
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
    /// 設定ファイル。足りない項目を書き足すのも、ここだけである。
    settings: PathBuf,
}

impl Service {
    pub fn new(system: MemoryDict, user: UserDict, settings: PathBuf) -> Self {
        Self {
            system,
            user,
            settings,
        }
    }

    /// 一つの頼みに答える。
    pub fn handle(&mut self, request: Request) -> (Response, Next) {
        match request {
            Request::Search(query) => (Response::Ok(self.search(&query)), Next::Listen),
            Request::Complete { prefix, limit } => {
                (Response::Ok(self.complete(&prefix, limit)), Next::Listen)
            }
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
            Request::Settings => (self.settings(), Next::Listen),
            Request::Save => (self.save(), Next::Listen),
            // 答えてから畳む。頼んだ側は「聞き届けた」ことを知れる。
            Request::Exit => (self.save(), Next::Stop),
        }
    }

    /// ユーザー辞書を先に、静的辞書を後に引く。
    ///
    /// 順番がそのまま候補の並びになる。**一度選んだ語が先に出る**のは
    /// この順番による。
    fn search(&self, query: &Query) -> Vec<Candidate> {
        let mut candidates = self.user.dict().lookup(query);
        for candidate in self.system.lookup(query) {
            if !candidates.iter().any(|seen| seen.word == candidate.word) {
                candidates.push(candidate);
            }
        }
        candidates
    }

    /// 前方一致する見出しを返す。
    ///
    /// **ユーザー辞書を先に、静的辞書を後に。** 前者は使った順、後者は
    /// 辞書順である。「かん」で静的辞書を引けば「かんあけ」から並ぶが、
    /// 直前に使った「かんじ」のほうが要る見込みが高い。
    ///
    /// 見出しを候補として返す。補完が返すのは**引くための見出し**であって、
    /// 変換の結果ではない。
    fn complete(&self, prefix: &str, limit: usize) -> Vec<Candidate> {
        let mut found: Vec<String> = self
            .user
            .dict()
            .complete_recent(prefix, limit)
            .into_iter()
            .map(str::to_owned)
            .collect();

        for key in self.system.complete(prefix, limit) {
            if found.len() >= limit {
                break;
            }
            if !found.iter().any(|seen| seen == key) {
                found.push(key.to_owned());
            }
        }
        found.into_iter().map(Candidate::new).collect()
    }

    /// 設定ファイルを読んで返す。
    ///
    /// **頼まれるたびに読み直す。** 利用者が書き換えたものが、入力先を
    /// 切り替えたときに効く。ファイルは小さいので、読み直しても障らない。
    ///
    /// 足りない項目があれば、ここで書き足す。書き手をサーバ一つに絞る
    /// ためで、TIP はファイルに触れない (隔離された入れ物の中からは、
    /// そもそも読めない)。
    ///
    /// 返すのは**ファイルの全文**である。読み方は受け取った側も同じ
    /// crate で揃えてあるので、値に崩して運び直す必要がない。
    fn settings(&self) -> Response {
        match crystalskk_settings::load(&self.settings) {
            Ok(loaded) => {
                if loaded.created {
                    eprintln!(
                        "crystalskk-server: 設定ファイルを作りました: {}",
                        self.settings.display()
                    );
                }
                if !loaded.added.is_empty() {
                    eprintln!(
                        "crystalskk-server: 設定ファイルに書き足しました: {}",
                        loaded.added.join(", ")
                    );
                }
                if !loaded.unknown.is_empty() {
                    eprintln!(
                        "crystalskk-server: 知らない設定があります: {}",
                        loaded.unknown.join(", ")
                    );
                }
                Response::Settings(loaded.text)
            }
            Err(e) => Response::Error(e.to_string()),
        }
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
        let settings = path.with_extension("toml");
        let _ = std::fs::remove_file(&settings);
        Service::new(system, UserDict::new(path), settings)
    }

    fn service() -> Service {
        service_with("かんじ /漢字/感じ/\n")
    }

    fn words(response: &Response) -> Vec<String> {
        match response {
            Response::Ok(candidates) => candidates.iter().map(|c| c.word.clone()).collect(),
            Response::Error(reason) => panic!("失敗した: {reason}"),
            Response::Settings(_) => panic!("候補ではなく設定が返った"),
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
    fn completion_puts_what_was_used_before_the_rest() {
        // **使った語が先に出る。** 辞書順に並べても、要る語が先に来る
        // 保証はない。
        let mut service = service_with("かんじ /漢字/\nかんじゃ /患者/\nかんき /寒気/\n");
        service.handle(Request::Learn {
            query: Query::okuri_nashi("かんじゃ"),
            word: "患者".to_owned(),
        });

        let (response, _) = service.handle(Request::Complete {
            prefix: "かん".to_owned(),
            limit: 16,
        });
        assert_eq!(words(&response), vec!["かんじゃ", "かんき", "かんじ"]);
    }

    #[test]
    fn completion_does_not_repeat_a_heading() {
        let mut service = service_with("かんじ /漢字/\n");
        service.handle(Request::Learn {
            query: Query::okuri_nashi("かんじ"),
            word: "漢字".to_owned(),
        });

        let (response, _) = service.handle(Request::Complete {
            prefix: "かん".to_owned(),
            limit: 16,
        });
        assert_eq!(words(&response), vec!["かんじ"], "両方に居ても一度だけ");
    }

    #[test]
    fn completion_never_offers_what_is_already_typed() {
        let mut service = service_with("かんじ /漢字/\n");
        let (response, _) = service.handle(Request::Complete {
            prefix: "かんじ".to_owned(),
            limit: 16,
        });
        assert!(words(&response).is_empty(), "打ち終えた見出しは出さない");
    }

    #[test]
    fn settings_come_back_as_the_whole_file() {
        // ファイルが無ければ雛形から作り、その全文を返す。
        let mut service = service();
        let (response, _) = service.handle(Request::Settings);
        let Response::Settings(text) = response else {
            panic!("設定が返らない: {response:?}");
        };
        assert!(crystalskk_settings::parse(&text).is_ok());
        assert!(service.settings.exists(), "ファイルができている");
        let _ = std::fs::remove_file(&service.settings);
    }

    #[test]
    fn a_broken_settings_file_is_explained() {
        let mut service = service();
        std::fs::write(&service.settings, "[completion\n").unwrap();
        let (response, next) = service.handle(Request::Settings);
        assert!(matches!(response, Response::Error(_)), "{response:?}");
        assert_eq!(next, Next::Listen, "設定が読めなくても辞書は引ける");
        let _ = std::fs::remove_file(&service.settings);
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
