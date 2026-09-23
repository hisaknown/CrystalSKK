//! 頼みに答える係。
//!
//! **辞書を持っているのはここだけである。** TIP は引きたいものを頼み、
//! 返ってきたものを使う。ユーザー辞書の書き手も一つに絞られ、学習が
//! 競り合って壊れることがなくなる (ADR-0016)。
//!
//! ここには Windows が出てこない。パイプの向こうから来た一行をどう
//! 解釈するか、それだけを担う。**運び方と、答え方を分けてある。**

use std::path::{Path, PathBuf};
use std::time::Duration;

use crystalskk_core::dict::{Candidate, CandidateSource, Context, NoopRanker, Query, Ranker};
use crystalskk_dict::UserDict;

use crate::library::Library;
use crystalskk_ipc::{Request, Reset, Response};

/// 頼みを聞き終えたあと、どうするか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// 待ち続ける。
    Listen,
    /// 畳む。
    Stop,
}

/// 辞書を持ち、頼みに答える係。
pub struct Service {
    /// 設定に並べた辞書。読むだけ。**一つの辞書であるかのように引く。**
    library: Library,
    /// ユーザー辞書。**この機械で唯一の書き手がここにいる。**
    user: UserDict,
    /// 設定ファイル。足りない項目を書き足すのも、ここだけである。
    settings: PathBuf,
    /// 変換の候補を、前後の文章から並べる (ADR-0030)。
    ranker: Box<dyn Ranker>,
    /// いまのランカーを作った設定。変わったときだけ作り直す。言語モデルの
    /// 読み込みは重い。
    ranker_settings: Option<crystalskk_settings::Ranker>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("library", &self.library)
            .field("user", &self.user)
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl Service {
    pub fn new(library: Library, user: UserDict, settings: PathBuf) -> Self {
        Self {
            library,
            user,
            settings,
            ranker: Box::new(NoopRanker),
            ranker_settings: None,
        }
    }

    /// 変換の候補を並べるものを差し替える。
    pub fn set_ranker(&mut self, ranker: Box<dyn Ranker>) {
        self.ranker = ranker;
    }

    /// 起動したときに、設定に並べた辞書を用意し始める。
    ///
    /// 取得には時間がかかるので、最初の頼みが来る前に始めておく。設定が
    /// 読めなければ何もしない (ユーザー辞書だけで答える)。
    pub fn prepare(&mut self) {
        if let Err(e) = self.read_settings() {
            eprintln!("crystalskk-server: {e}");
        }
    }

    /// 一つの頼みに答える。
    pub fn handle(&mut self, request: Request) -> (Response, Next) {
        // 裏で読み終えた辞書があれば、答える前に差し替える。
        self.library.poll();
        match request {
            Request::Search(query) => (self.search(&query), Next::Listen),
            Request::Convert {
                query,
                before,
                after,
            } => (self.convert(&query, before, after), Next::Listen),
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
            Request::Reset(what) => (self.reset(what), Next::Listen),
            Request::OpenFolder => (self.open_folder(), Next::Listen),
            Request::Save => (self.save(), Next::Listen),
            // 答えてから畳む。頼んだ側は「聞き届けた」ことを知れる。
            Request::Exit => (self.save(), Next::Stop),
        }
    }

    /// ユーザー辞書を先に、静的辞書を後に引く。
    ///
    /// 順番がそのまま候補の並びになる。**一度選んだ語が先に出る**のは
    /// この順番による。
    ///
    /// 辞書をまだ一度も読み終えていなければ、**引けなかった**と答える。
    /// 「無い」と答えると、知っているはずの語で辞書登録が始まる。
    fn search(&self, query: &Query) -> Response {
        if let Some(reason) = self.library.waiting() {
            return Response::Error(reason);
        }
        let mut candidates = self.user.dict().lookup(query);
        for candidate in self.library.lookup(query) {
            if !candidates.iter().any(|seen| seen.word == candidate.word) {
                candidates.push(candidate);
            }
        }
        // 欠けた辞書があれば、候補が無いことを「無い」とは言い切れない
        // (ADR-0029)。
        if candidates.is_empty()
            && let Some(reason) = self.library.shortfall()
        {
            return Response::Error(reason);
        }
        Response::Ok(candidates)
    }

    /// 変換のために引き、前後の文章から並べる。
    ///
    /// **最後に使った語は先頭から動かさない。** ユーザー辞書の先頭がそれで
    /// ある。利用者の手癖を壊さないためで、「受け取り/受取」のような、
    /// 文脈では決まらない書き分けもここで片付く。並べるのは残りだけ。
    fn convert(&self, query: &Query, before: String, after: String) -> Response {
        let mut candidates = match self.search(query) {
            Response::Ok(candidates) => candidates,
            other => return other,
        };
        let pinned = usize::from(!self.user.dict().lookup(query).is_empty());
        if candidates.len() > pinned + 1 {
            let context = Context {
                preceding_text: Some(before),
                following_text: Some(after),
                ..Context::default()
            };
            let mut rest = candidates.split_off(pinned);
            self.ranker.rank(&context, query, &mut rest);
            candidates.append(&mut rest);
        }
        Response::Ok(candidates)
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

        for key in self.library.complete(prefix, limit) {
            if found.len() >= limit {
                break;
            }
            if !found.contains(&key) {
                found.push(key);
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
    fn settings(&mut self) -> Response {
        match self.read_settings() {
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
                if loaded.romaji_created {
                    eprintln!(
                        "crystalskk-server: ローマ字テーブルを作りました: {}",
                        loaded.romaji_path.display()
                    );
                }
                Response::Settings {
                    config: loaded.text,
                    romaji: loaded.romaji_text,
                }
            }
            Err(e) => Response::Error(e.to_string()),
        }
    }

    /// 設定を読み、並べた辞書に合わせる。
    ///
    /// 並びもファイルも変わっていなければ、辞書には何もしない。
    fn read_settings(&mut self) -> Result<crystalskk_settings::Loaded, crystalskk_settings::Error> {
        let loaded = crystalskk_settings::load(&self.settings)?;
        let directory = self
            .settings
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        self.library
            .configure(&loaded.settings.dictionaries, &directory);
        let ranker_dir = ranker_dir();
        self.configure_ranker(&loaded.settings.ranker, &ranker_dir);
        Ok(loaded)
    }

    /// 設定が変わっていれば、ランカーを作り直す。
    ///
    /// **作れなくても入力は止めない。** 記録に残し、辞書の順で答え続ける。
    fn configure_ranker(&mut self, settings: &crystalskk_settings::Ranker, ranker_dir: &Path) {
        if self.ranker_settings.as_ref() == Some(settings) {
            return;
        }
        self.ranker_settings = Some(settings.clone());
        self.ranker = match language_model(settings, ranker_dir) {
            Ok(Some(ranker)) => ranker,
            Ok(None) => Box::new(NoopRanker),
            Err(e) => {
                eprintln!("crystalskk-server: 候補を並べ替えずに続けます: {e}");
                Box::new(NoopRanker)
            }
        };
    }

    /// 雛形で上書きする。**元の中身は隣に退避する。**
    ///
    /// 上書きするのは利用者に頼まれたときだけである。版を上げても、
    /// ローマ字テーブルには触れない (ADR-0021)。
    fn reset(&self, what: Reset) -> Response {
        let outcome = match what {
            Reset::Settings => crystalskk_settings::reset_settings(&self.settings)
                .map(|backup| (self.settings.clone(), backup)),
            Reset::Romaji => crystalskk_settings::reset_romaji(&self.settings),
        };
        match outcome {
            Ok((target, backup)) => {
                let name = file_name(&target);
                let told = match backup {
                    Some(backup) => format!(
                        "{name} を雛形で上書きしました。元の中身は {} に残しました。",
                        file_name(&backup)
                    ),
                    None => format!("{name} を雛形から作りました。"),
                };
                eprintln!("crystalskk-server: {told}");
                Response::Done(told)
            }
            Err(e) => Response::Error(e.to_string()),
        }
    }

    /// 設定ファイルの置き場所をエクスプローラーで開く。
    fn open_folder(&self) -> Response {
        let Some(folder) = self.settings.parent() else {
            return Response::Error("設定ファイルの置き場所が分かりません".to_owned());
        };
        match crate::shell::open(folder) {
            Ok(()) => Response::Done(format!("{} を開きました。", folder.display())),
            Err(e) => Response::Error(format!("{} を開けません: {e}", folder.display())),
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

/// 見せる名前。場所まで出すと長くなる。
fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// 言語モデル一式の置き場所。この実行ファイルの隣の `ranker` (ADR-0031)。
fn ranker_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    crate::paths::ranker_dir(exe.parent().unwrap_or(Path::new(".")))
}

/// 設定どおりの、言語モデルで並べるランカー。切ってあれば `None`。
///
/// 言語モデルは `ranker_dir` に導入されたものを使う。選べない (ADR-0031)。
fn language_model(
    settings: &crystalskk_settings::Ranker,
    ranker_dir: &Path,
) -> Result<Option<Box<dyn Ranker>>, String> {
    use crate::paths::{RANKER_MODEL, RANKER_RUNTIME, RANKER_TOKENIZER};

    if !settings.enabled {
        return Ok(None);
    }
    let scorer = crystalskk_lm::LlamaScorer::load(
        &ranker_dir.join(RANKER_RUNTIME),
        &ranker_dir.join(RANKER_MODEL),
        &ranker_dir.join(RANKER_TOKENIZER),
        settings.threads,
    )?;
    let policy = crystalskk_lm::Policy {
        weight: settings.weight.0,
        deadline: Duration::from_millis(u64::from(settings.deadline_ms)),
        before: settings.before,
        after: settings.after,
    };
    Ok(Some(Box::new(crystalskk_lm::LmRanker::new(scorer, policy))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 試験用。ユーザー辞書と設定は試験ごとの置き場所に置き、書き出しても
    /// 実害が出ないようにする。ローマ字テーブルは設定ファイルの隣に
    /// 作られるので、**置き場所ごと分けないと試験どうしで取り合う。**
    fn service_with(entries: &str) -> Service {
        let directory = std::env::temp_dir().join(format!(
            "crystalskk-server-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("置き場所を作れる");

        // 辞書は手元のファイルとして並べる。**試験はネットワークに出ない。**
        let dictionary = directory.join("system.dict");
        std::fs::write(&dictionary, entries).unwrap();
        let mut library = Library::new(directory.join("cache"), refuse);
        library.configure(
            &[crystalskk_settings::Source::File(
                dictionary.display().to_string(),
            )],
            &directory,
        );
        library.settle();

        Service::new(
            library,
            UserDict::new(directory.join("user.dict")),
            directory.join("config.toml"),
        )
    }

    fn refuse(_url: &str, _path: &std::path::Path) -> Result<bool, String> {
        Err("試験では通信しない".to_owned())
    }

    fn service() -> Service {
        service_with("かんじ /漢字/感じ/\n")
    }

    fn words(response: &Response) -> Vec<String> {
        match response {
            Response::Ok(candidates) => candidates.iter().map(|c| c.word.clone()).collect(),
            Response::Error(reason) => panic!("失敗した: {reason}"),
            Response::Settings { .. } | Response::Done(_) => {
                panic!("候補ではないものが返った: {response:?}")
            }
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
    fn nothing_found_while_a_dictionary_is_missing_is_not_an_answer() {
        // 欠けた辞書にあったはずの語で、登録を始めさせない (ADR-0029)。
        let mut service = service();
        let directory = std::env::temp_dir();
        service.library.configure(
            &[crystalskk_settings::Source::File(
                directory
                    .join("crystalskk-no-such.dict")
                    .display()
                    .to_string(),
            )],
            &directory,
        );
        service.library.settle();
        let (response, _) = service.handle(Request::Search(Query::okuri_nashi("ない")));
        assert!(
            matches!(&response, Response::Error(reason) if reason.contains("登録には進みません")),
            "{response:?}"
        );
    }

    /// 前の文章の最後の文字を含む候補を先に出す、試験用のランカー。
    struct Echo;

    impl Ranker for Echo {
        fn rank(&self, context: &Context, _query: &Query, candidates: &mut Vec<Candidate>) {
            let last = context
                .preceding_text
                .as_deref()
                .and_then(|text| text.chars().last());
            candidates.sort_by_key(|c| !last.is_some_and(|last| c.word.contains(last)));
        }
    }

    fn convert(service: &mut Service, key: &str, before: &str) -> Vec<String> {
        let (response, _) = service.handle(Request::Convert {
            query: Query::okuri_nashi(key),
            before: before.to_owned(),
            after: String::new(),
        });
        words(&response)
    }

    #[test]
    fn a_conversion_is_ordered_by_the_ranker() {
        let mut service = service_with("かんじ /漢字/感じ/幹事/\n");
        service.set_ranker(Box::new(Echo));
        assert_eq!(
            convert(&mut service, "かんじ", "会議の幹"),
            ["幹事", "漢字", "感じ"]
        );
        // 並べるのは変換だけ。ただ引くときは辞書の順のまま。
        let (response, _) = service.handle(Request::Search(Query::okuri_nashi("かんじ")));
        assert_eq!(words(&response), ["漢字", "感じ", "幹事"]);
    }

    #[test]
    fn what_was_used_last_stays_first_in_a_conversion() {
        let mut service = service_with("かんじ /漢字/感じ/幹事/\n");
        service.set_ranker(Box::new(Echo));
        service.handle(Request::Learn {
            query: Query::okuri_nashi("かんじ"),
            word: "感じ".to_owned(),
        });
        assert_eq!(
            convert(&mut service, "かんじ", "会議の幹"),
            ["感じ", "幹事", "漢字"]
        );
    }

    fn ranker_settings(enabled: bool) -> crystalskk_settings::Ranker {
        crystalskk_settings::Ranker {
            enabled,
            weight: crystalskk_settings::Weight(1.0),
            deadline_ms: 100,
            before: 100,
            after: 5,
            threads: 4,
        }
    }

    #[test]
    fn no_language_model_is_loaded_while_the_ranker_is_off() {
        let ranker = language_model(&ranker_settings(false), Path::new("nowhere"));
        assert!(matches!(ranker, Ok(None)));
    }

    #[test]
    fn a_ranker_that_cannot_be_built_says_why() {
        let Err(e) = language_model(&ranker_settings(true), Path::new("nowhere")) else {
            panic!("作れないはず");
        };
        assert!(e.contains("nowhere"), "{e}");
    }

    #[test]
    fn conversions_go_on_when_the_ranker_cannot_be_built() {
        // **並べ替えられなくても入力は止めない。** 辞書の順で答える。
        let mut service = service_with("かんじ /漢字/感じ/幹事/\n");
        service.configure_ranker(&ranker_settings(true), Path::new("nowhere"));
        assert_eq!(
            convert(&mut service, "かんじ", "会議の幹"),
            ["漢字", "感じ", "幹事"]
        );
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
    fn completion_offers_what_is_already_typed_first() {
        // 打ち終えた見出しが辞書にあれば、それが最初の補完候補になる。
        let mut service = service_with("かんじ /漢字/\nかんじゃ /患者/\n");
        let (response, _) = service.handle(Request::Complete {
            prefix: "かんじ".to_owned(),
            limit: 16,
        });
        assert_eq!(words(&response), vec!["かんじ", "かんじゃ"]);
    }

    #[test]
    fn settings_come_back_as_the_whole_file() {
        // ファイルが無ければ雛形から作り、その全文を返す。
        let mut service = service();
        let (response, _) = service.handle(Request::Settings);
        let Response::Settings { config, romaji } = response else {
            panic!("設定が返らない: {response:?}");
        };
        assert!(crystalskk_settings::parse(&config, &romaji).is_ok());
        assert!(service.settings.exists(), "ファイルができている");
        let _ = std::fs::remove_dir_all(service.settings.parent().unwrap());
    }

    #[test]
    fn a_broken_settings_file_is_explained() {
        let mut service = service();
        std::fs::write(&service.settings, "[completion\n").unwrap();
        let (response, next) = service.handle(Request::Settings);
        assert!(matches!(response, Response::Error(_)), "{response:?}");
        assert_eq!(next, Next::Listen, "設定が読めなくても辞書は引ける");
        let _ = std::fs::remove_dir_all(service.settings.parent().unwrap());
    }

    #[test]
    fn resetting_tells_where_the_old_one_went() {
        let mut service = service();
        service.handle(Request::Settings);
        let table = service.settings.with_file_name("romaji.txt");
        std::fs::write(&table, "ka\tか\n").unwrap();

        let (response, _) = service.handle(Request::Reset(Reset::Romaji));
        let Response::Done(told) = response else {
            panic!("上書きできない: {response:?}");
        };
        assert!(told.contains("romaji.txt.bak"), "{told}");
        assert_eq!(
            std::fs::read_to_string(&table).unwrap(),
            crystalskk_settings::ROMAJI_TEMPLATE
        );
        let _ = std::fs::remove_dir_all(service.settings.parent().unwrap());
    }

    #[test]
    fn nothing_is_called_missing_while_the_dictionaries_are_on_their_way() {
        // 取得しているあいだに「無い」と答えると、辞書登録が始まってしまう。
        fn slow(_url: &str, _path: &std::path::Path) -> Result<bool, String> {
            std::thread::sleep(std::time::Duration::from_millis(300));
            Err("届かない".to_owned())
        }
        let directory = std::env::temp_dir().join(format!(
            "crystalskk-server-test-waiting-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let mut library = Library::new(directory.join("cache"), slow);
        library.configure(
            &[crystalskk_settings::Source::Url(
                "https://example.com/a".to_owned(),
            )],
            &directory,
        );
        let mut service = Service::new(
            library,
            UserDict::new(directory.join("user.dict")),
            directory.join("config.toml"),
        );
        let (response, _) = service.handle(Request::Search(Query::okuri_nashi("かんじ")));
        assert!(matches!(response, Response::Error(_)), "{response:?}");
        let _ = std::fs::remove_dir_all(&directory);
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
