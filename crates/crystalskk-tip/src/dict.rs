//! 辞書の用意。
//!
//! # これは暫定である
//!
//! TIP は入力先アプリのプロセスの中で動く。辞書をここで読むということは、
//! **Word にも Chrome にも辞書の写しが一つずつ載る**ということである。
//! PRD §7 が変換をサーバープロセスへ追い出すと決めたのは、まさにこれを
//! 避けるためだった。
//!
//! サーバーはまだない。それまでの繋ぎとして、ここで読む。
//!
//! 代償を小さくするため、辞書は**引かれるまで読まない**。ほとんどの打鍵は
//! 辞書を必要としないので、変換しないまま終わるアプリでは一度も読まれない。

use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use crystalskk_core::Engine;
use crystalskk_core::dict::{Candidate, CandidateSource, ChainedSource, Query};
use crystalskk_dict::{MemoryDict, UserDict, encoding, paths};

use crate::log;

/// 引かれるまで読まない静的辞書。
struct LazyDict {
    /// 一度だけ読む。読めなければ空のまま。
    loaded: OnceCell<MemoryDict>,
}

impl LazyDict {
    fn new() -> Self {
        Self {
            loaded: OnceCell::new(),
        }
    }

    fn dict(&self) -> &MemoryDict {
        self.loaded.get_or_init(|| {
            let Ok(path) = paths::system_dictionary() else {
                log::error("辞書の置き場所が分からない");
                return MemoryDict::new();
            };
            let Ok(bytes) = std::fs::read(&path) else {
                log::error(&format!("辞書がない: {}", path.display()));
                return MemoryDict::new();
            };

            let decoded = encoding::decode(&bytes);
            let (dict, report) = MemoryDict::parse(&decoded.text);
            log::write(&format!(
                "辞書を読んだ: {} 件 ({}, 読み飛ばし {} 行)",
                report.entries, decoded.encoding, report.skipped
            ));
            dict
        })
    }
}

impl CandidateSource for LazyDict {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.dict().lookup(query)
    }
}

/// 引きながら書き換えられるユーザー辞書。
///
/// エンジンは候補ソースを所有するが、学習ではそれを書き換える必要がある。
/// エンジンに書き換えの口を持たせるより、共有の持ち手をこちら側で用意する
/// ほうが、エンジンを純粋に保てる。
#[derive(Clone)]
pub struct SharedUserDict(Rc<RefCell<UserDict>>);

impl SharedUserDict {
    fn load() -> Self {
        let path = paths::user_dictionary().unwrap_or_default();
        let dict = match UserDict::load(&path) {
            Ok((dict, report)) => {
                log::write(&format!("ユーザー辞書を読んだ: {} 件", report.entries));
                dict
            }
            Err(e) => {
                log::error(&format!("ユーザー辞書を読めなかった: {e}"));
                UserDict::new(path)
            }
        };
        Self(Rc::new(RefCell::new(dict)))
    }

    /// 確定した語を学習する。辞書登録も同じ操作になる。
    pub fn learn(&self, query: &Query, word: &str) {
        self.0.borrow_mut().learn(query, word);
    }

    /// 変更があれば書き出す。
    pub fn save(&self) {
        let mut dict = self.0.borrow_mut();
        if !dict.is_dirty() {
            return;
        }
        match dict.save() {
            Ok(()) => log::write("ユーザー辞書を保存した"),
            Err(e) => log::error(&format!("ユーザー辞書を保存できなかった: {e}")),
        }
    }
}

impl std::fmt::Debug for SharedUserDict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedUserDict").finish_non_exhaustive()
    }
}

impl CandidateSource for SharedUserDict {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.0.borrow().lookup(query)
    }
}

/// 辞書を繋いだエンジンと、学習の書き込み先を作る。
///
/// ユーザー辞書を先に置くのは、学習した語を先に出すため。
pub fn build() -> (Engine, SharedUserDict) {
    let user = SharedUserDict::load();
    let sources: Vec<Box<dyn CandidateSource>> =
        vec![Box::new(user.clone()), Box::new(LazyDict::new())];
    let engine = Engine::new(Box::new(ChainedSource::new(sources)));
    (engine, user)
}
