//! 辞書の一揃い。
//!
//! 設定に並べた辞書を、**一つのファイルに結合したかのように**引く
//! (ADR-0022)。ファイルとして結合はしない。辞書ごとに更新の頻度も
//! 使用許諾も違うので、別々に持っておくほうが扱いやすい。
//!
//! - 引くときは並べた順に引き、同じ語は先に出たほうにまとめる。
//! - 補完は全部の辞書から集め、見出しの辞書順に並べ直す。一つのファイル
//!   だったら、そう並んでいる。
//!
//! # 取得と読み込みは裏でやる
//!
//! L 辞書の取得には数秒かかる。そのあいだ頼みを待たせるわけにはいかない
//! ので、裏のスレッドで取得して読み込み、できたら差し替える。
//!
//! 初めての読み込みが終わるまでは、[`Library::waiting`] が理由を返す。
//! 引く側はそれを「引けなかった」として伝える。**「辞書に無い」と取り
//! 違えて辞書登録が始まるのを防ぐ。** 二度目からは、読み直しているあいだも
//! 前の辞書で答える。
//!
//! # 欠けた辞書 (ADR-0029)
//!
//! 取得できず、手元にも前の版が無い辞書は**欠けている**。欠けているあいだは
//! [`Library::shortfall`] が理由を返す。引く側は、候補が一つも無かったときに
//! それを伝え、辞書登録には進ませない。**欠けた辞書にあったはずの語で登録が
//! 始まるのを防ぐ。** 欠けた URL の辞書は、しばらく置いて取り直す。
//!
//! 更新を確かめられなかっただけなら、前の版で引けるので、記録に残すだけに
//! する。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use crystalskk_core::dict::{Candidate, CandidateSource, Query};
use crystalskk_dict::{MemoryDict, derive, encoding};
use crystalskk_settings::Source;

/// URL から取ってきて、手元の場所に置く。変わっていれば真。
///
/// 試験では差し替える。**試験がネットワークに出てはいけない。**
pub type Fetch = fn(url: &str, path: &Path) -> Result<bool, String>;

/// 欠けた辞書を取り直すまでの間。
#[cfg(not(test))]
const RETRY: Duration = Duration::from_secs(5 * 60);
#[cfg(test)]
const RETRY: Duration = Duration::ZERO;

/// 本物の取得。
pub fn fetch_over_http(url: &str, path: &Path) -> Result<bool, String> {
    crystalskk_fetch::refresh(url, path)
        .map(|installed| installed.is_some())
        .map_err(|e| e.to_string())
}

/// 辞書の一揃い。
#[derive(Debug)]
pub struct Library {
    /// URL の辞書を置く場所。
    cache: PathBuf,
    fetch: Fetch,
    /// 引いている辞書。並べた順。
    shelves: Vec<Shelf>,
    /// いま載っている辞書が、どの計画で読まれたものか。まだ何も読んで
    /// いなければ `None`。
    loaded: Option<Vec<Entry>>,
    /// 裏で進んでいる読み込み。
    job: Option<Job>,
    /// 読み込みの通し番号。古い読み込みの結果を捨てるのに使う。
    generation: u64,
    /// この起動で、URL の辞書の更新を確かめたか。
    checked_updates: bool,
    /// 直近の読み込みで困ったこと。
    problems: Vec<String>,
    /// 欠けている辞書と、その訳。
    missing: Vec<String>,
    /// 最後に欠けていると分かった時刻。取り直すのに使う。
    missing_since: Option<Instant>,
}

#[derive(Debug)]
struct Shelf {
    /// 書かれたままの在りか。知らせに使う。
    name: String,
    dict: MemoryDict,
}

/// 一つの辞書をどこから読むか。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    written: String,
    path: PathBuf,
    url: Option<String>,
    /// 読んだ辞書からカタカナ語の辞書を作るか (ADR-0023)。
    katakana: bool,
    /// 読んだときのファイルの時刻。変わっていれば読み直す。
    stamp: Option<SystemTime>,
}

impl Entry {
    /// 時刻を除いた、どこから読むかだけの形。
    fn place(&self) -> (&str, &Path, Option<&str>, bool) {
        (
            &self.written,
            &self.path,
            self.url.as_deref(),
            self.katakana,
        )
    }
}

#[derive(Debug)]
struct Job {
    generation: u64,
    places: Vec<Entry>,
    receiver: mpsc::Receiver<Outcome>,
}

#[derive(Debug)]
struct Outcome {
    generation: u64,
    shelves: Vec<Shelf>,
    /// 読み終えたときの時刻を入れた計画。
    entries: Vec<Entry>,
    problems: Vec<String>,
    /// 読めなかった辞書と、その訳。
    missing: Vec<String>,
}

impl Library {
    pub fn new(cache: PathBuf, fetch: Fetch) -> Self {
        Self {
            cache,
            fetch,
            shelves: Vec::new(),
            loaded: None,
            job: None,
            generation: 0,
            checked_updates: false,
            problems: Vec::new(),
            missing: Vec::new(),
            missing_since: None,
        }
    }

    /// 並べられた辞書に合わせる。**変わっていなければ何もしない。**
    ///
    /// 設定を尋ねられるたびに呼ぶ (入力先を切り替えるたび)。並びが
    /// 変わったか、ファイルが書き換えられていれば、裏で読み直す。
    pub fn configure(&mut self, sources: &[Source], settings_directory: &Path) {
        let mut problems = Vec::new();
        let entries: Vec<Entry> = sources
            .iter()
            .filter_map(|source| match self.locate(source, settings_directory) {
                Ok(entry) => Some(entry),
                Err(problem) => {
                    problems.push(problem);
                    None
                }
            })
            .collect();

        let same_places = |a: &[Entry], b: &[Entry]| {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.place() == y.place())
        };
        if let Some(job) = &self.job
            && same_places(&job.places, &entries)
        {
            return;
        }
        if self.loaded.as_deref() == Some(entries.as_slice()) {
            // 欠けた URL の辞書があれば、しばらく置いて取り直す。
            let fetchable = entries.iter().any(|e| e.url.is_some() && !e.path.exists());
            let due = self.missing_since.is_some_and(|at| at.elapsed() >= RETRY);
            if fetchable && due && self.job.is_none() {
                self.missing_since = None;
                self.start(entries, problems, false);
            }
            return;
        }

        // 更新を確かめるのは、起動して最初の一度だけ。**入力先を切り替える
        // たびに通信はしない。**
        let check_updates = !self.checked_updates;
        self.checked_updates = true;
        self.start(entries, problems, check_updates);
    }

    /// 裏の読み込みが終わっていれば、差し替える。頼みに答える前に呼ぶ。
    pub fn poll(&mut self) {
        let Some(job) = &self.job else {
            return;
        };
        let outcome = match job.receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                // 読み込みが途中で倒れた。前の辞書のまま続ける。
                eprintln!("crystalskk-server: 辞書の読み込みが途中で止まりました");
                self.job = None;
                return;
            }
        };
        if outcome.generation != job.generation {
            return;
        }
        for problem in &outcome.problems {
            eprintln!("crystalskk-server: {problem}");
        }
        for shelf in &outcome.shelves {
            eprintln!("crystalskk-server: 辞書を読みました: {}", shelf.name);
        }
        self.shelves = outcome.shelves;
        self.loaded = Some(outcome.entries);
        self.problems = outcome.problems;
        self.missing_since = (!outcome.missing.is_empty()).then(Instant::now);
        self.missing = outcome.missing;
        self.job = None;
    }

    /// まだ一度も読み終えていないなら、その理由。
    ///
    /// 引く側はこれを「引けなかった」として伝える。**「辞書に無い」と
    /// 取り違えると、辞書登録が始まってしまう。**
    pub fn waiting(&self) -> Option<String> {
        if self.loaded.is_some() {
            return None;
        }
        let job = self.job.as_ref()?;
        let fetching = job
            .places
            .iter()
            .any(|e| e.url.is_some() && !e.path.exists());
        Some(if fetching {
            "辞書を取得しています。しばらくお待ちください".to_owned()
        } else {
            "辞書を読んでいます".to_owned()
        })
    }

    /// 欠けている辞書があれば、その知らせ。
    ///
    /// 引く側は、候補が一つも無かったときにこれを伝える。**欠けた辞書に
    /// あったはずの語で、辞書登録を始めさせない。**
    pub fn shortfall(&self) -> Option<String> {
        if self.missing.is_empty() {
            return None;
        }
        Some(format!(
            "{}。辞書に無い語なのか分からないので、登録には進みません",
            self.missing.join("、")
        ))
    }

    /// 直近の読み込みで困ったこと。
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// 並べた順に引き、同じ語は先に出たほうにまとめる。
    pub fn lookup(&self, query: &Query) -> Vec<Candidate> {
        let mut found: Vec<Candidate> = Vec::new();
        for shelf in &self.shelves {
            for candidate in shelf.dict.lookup(query) {
                if !found.iter().any(|seen| seen.word == candidate.word) {
                    found.push(candidate);
                }
            }
        }
        found
    }

    /// 前方一致する見出しを、全部の辞書から辞書順に集める。
    pub fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        let mut keys: Vec<&str> = self
            .shelves
            .iter()
            .flat_map(|shelf| shelf.dict.complete(prefix, limit))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys.truncate(limit);
        keys.into_iter().map(str::to_owned).collect()
    }

    /// 在りかを、読む場所に解く。
    ///
    /// 派生した辞書は、元の辞書の在りかを読む。取得も置き場所も元の辞書と
    /// 同じなので、並びに両方あっても取るのは一度で済む。
    fn locate(&self, source: &Source, settings_directory: &Path) -> Result<Entry, String> {
        let written = source.as_written();
        let (path, url) = match source.base() {
            Source::Url(url) => {
                let path = crystalskk_fetch::cache_path(&self.cache, url)
                    .map_err(|e| format!("{written}: {e}"))?;
                (path, Some(url.clone()))
            }
            _ => {
                let path = source
                    .resolve(settings_directory)
                    .expect("ファイルの在りかは必ず解ける");
                (path, None)
            }
        };
        let stamp = modified(&path);
        Ok(Entry {
            written,
            path,
            url,
            katakana: source.is_katakana(),
            stamp,
        })
    }

    /// 裏で読み込みを始める。前の読み込みが残っていれば、その結果は捨てる。
    fn start(&mut self, entries: Vec<Entry>, problems: Vec<String>, check_updates: bool) {
        self.generation += 1;
        let generation = self.generation;
        let (sender, receiver) = mpsc::channel();
        let fetch = self.fetch;
        let places = entries.clone();
        std::thread::spawn(move || {
            let mut outcome = load_all(entries, fetch, check_updates);
            outcome.generation = generation;
            // 在りかが解けなかった辞書も、欠けている。
            outcome.missing.splice(0..0, problems.iter().cloned());
            outcome.problems.splice(0..0, problems);
            // 受け手が居なくなっていれば (次の読み込みに替わった) 捨てる。
            let _ = sender.send(outcome);
        });
        self.job = Some(Job {
            generation,
            places,
            receiver,
        });
    }

    /// 試験で、読み込みが終わるのを待つ。
    #[cfg(test)]
    pub(crate) fn settle(&mut self) {
        for _ in 0..500 {
            self.poll();
            if self.job.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("辞書の読み込みが終わらない");
    }
}

/// 裏のスレッドで、並べた辞書を順に用意して読む。
fn load_all(mut entries: Vec<Entry>, fetch: Fetch, check_updates: bool) -> Outcome {
    let mut shelves = Vec::new();
    let mut problems = Vec::new();
    let mut missing = Vec::new();
    // 同じファイルは一度だけ取って、一度だけ読む。L 辞書とそこから作る
    // カタカナ語の辞書は、同じファイルを読む。
    let mut fetched: Vec<PathBuf> = Vec::new();
    let mut texts: HashMap<PathBuf, String> = HashMap::new();

    for entry in &mut entries {
        let mut fetch_failure = None;
        if let Some(url) = &entry.url
            && !fetched.contains(&entry.path)
            && (check_updates || !entry.path.exists())
        {
            fetched.push(entry.path.clone());
            match fetch(url, &entry.path) {
                Ok(true) => eprintln!("crystalskk-server: 辞書を取得しました: {url}"),
                Ok(false) => {}
                // 手元に前のものがあれば、それで続ける。通信できないのは
                // よくあることで、辞書が引けなくなるほどのことではない。
                Err(e) => {
                    let problem = format!("{} を取得できません: {e}", entry.written);
                    problems.push(problem.clone());
                    fetch_failure = Some(problem);
                }
            }
        }

        if !texts.contains_key(&entry.path) {
            match fs::read(&entry.path) {
                Ok(bytes) => {
                    let decoded = encoding::decode(&bytes);
                    texts.insert(entry.path.clone(), decoded.text);
                }
                Err(e) => {
                    let problem = format!("{} を読めません: {e}", entry.written);
                    problems.push(problem.clone());
                    // 取れなかったから読めないのなら、取れなかったほうを言う。
                    missing.push(fetch_failure.clone().unwrap_or(problem));
                }
            }
        }
        if let Some(text) = texts.get(&entry.path) {
            let (dict, _) = if entry.katakana {
                MemoryDict::parse(&derive::katakana_words(text))
            } else {
                MemoryDict::parse(text)
            };
            shelves.push(Shelf {
                name: entry.written.clone(),
                dict,
            });
        }
        entry.stamp = modified(&entry.path);
    }

    Outcome {
        generation: 0,
        shelves,
        entries,
        problems,
        missing,
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "crystalskk-library-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn refuse(_url: &str, _path: &Path) -> Result<bool, String> {
        Err("通信しない".to_owned())
    }

    /// 取得したことにして、小さな辞書を置く。
    fn pretend(_url: &str, path: &Path) -> Result<bool, String> {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "とりよせ /取寄/\n").unwrap();
        Ok(true)
    }

    fn words(candidates: &[Candidate]) -> Vec<&str> {
        candidates.iter().map(|c| c.word.as_str()).collect()
    }

    fn files(directory: &Path, names: &[&str]) -> Vec<Source> {
        names
            .iter()
            .map(|n| Source::File(directory.join(n).display().to_string()))
            .collect()
    }

    #[test]
    fn dictionaries_are_read_in_the_order_listed() {
        let directory = scratch("order");
        fs::write(directory.join("a.dict"), "かんじ /漢字/幹事/\n").unwrap();
        fs::write(directory.join("b.dict"), "かんじ /感じ/漢字/\nべつ /別/\n").unwrap();

        let mut library = Library::new(directory.join("cache"), refuse);
        library.configure(&files(&directory, &["a.dict", "b.dict"]), &directory);
        library.settle();

        // **一つの辞書だったかのように。** 並べた順で、重なる語は一度だけ。
        let found = library.lookup(&Query::okuri_nashi("かんじ"));
        assert_eq!(words(&found), ["漢字", "幹事", "感じ"]);
        assert_eq!(words(&library.lookup(&Query::okuri_nashi("べつ"))), ["別"]);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn completion_is_sorted_as_if_there_were_one_file() {
        let directory = scratch("complete");
        fs::write(directory.join("a.dict"), "かんじ /漢字/\nかんき /寒気/\n").unwrap();
        fs::write(directory.join("b.dict"), "かんい /簡易/\nかんじ /感じ/\n").unwrap();

        let mut library = Library::new(directory.join("cache"), refuse);
        library.configure(&files(&directory, &["a.dict", "b.dict"]), &directory);
        library.settle();

        assert_eq!(library.complete("かん", 16), ["かんい", "かんき", "かんじ"]);
        assert_eq!(library.complete("かん", 2), ["かんい", "かんき"]);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn nothing_is_answered_as_missing_before_the_first_load() {
        // 取得しているあいだに「無い」と答えると、辞書登録が始まってしまう。
        let directory = scratch("waiting");
        let mut library = Library::new(directory.join("cache"), pretend);
        assert_eq!(library.waiting(), None, "何も頼まれていなければ待たない");

        library.configure(
            &[Source::Url("https://example.com/a".to_owned())],
            &directory,
        );
        assert!(library.waiting().is_some(), "読み終えるまでは待つ");
        library.settle();
        assert_eq!(library.waiting(), None);
        let found = library.lookup(&Query::okuri_nashi("とりよせ"));
        assert_eq!(words(&found), ["取寄"]);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_unchanged_list_is_not_read_again() {
        let directory = scratch("unchanged");
        fs::write(directory.join("a.dict"), "かんじ /漢字/\n").unwrap();
        let mut library = Library::new(directory.join("cache"), refuse);
        let sources = files(&directory, &["a.dict"]);
        library.configure(&sources, &directory);
        library.settle();

        library.configure(&sources, &directory);
        assert!(
            library.job.is_none(),
            "入力先を切り替えるたびに読み直さない"
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_rewritten_file_is_read_again() {
        let directory = scratch("rewritten");
        let file = directory.join("a.dict");
        fs::write(&file, "かんじ /漢字/\n").unwrap();
        let mut library = Library::new(directory.join("cache"), refuse);
        let sources = files(&directory, &["a.dict"]);
        library.configure(&sources, &directory);
        library.settle();

        // 時刻が確実に変わるよう、少し先の時刻を付ける。
        fs::write(&file, "かんじ /感じ/\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(later)
            .unwrap();

        library.configure(&sources, &directory);
        library.settle();
        assert_eq!(
            words(&library.lookup(&Query::okuri_nashi("かんじ"))),
            ["感じ"]
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_dictionary_that_cannot_be_fetched_does_not_block_forever() {
        // 通信できなくても、ほかの辞書とユーザー辞書で使い続けられる。
        let directory = scratch("offline");
        fs::write(directory.join("a.dict"), "かんじ /漢字/\n").unwrap();
        let mut library = Library::new(directory.join("cache"), refuse);
        let mut sources = vec![Source::Url("https://example.com/far".to_owned())];
        sources.extend(files(&directory, &["a.dict"]));
        library.configure(&sources, &directory);
        library.settle();

        assert_eq!(library.waiting(), None);
        assert_eq!(
            words(&library.lookup(&Query::okuri_nashi("かんじ"))),
            ["漢字"]
        );
        assert!(
            library
                .problems()
                .iter()
                .any(|p| p.contains("example.com/far"))
        );
        // 手元に前の版も無いので、欠けている。
        let shortfall = library.shortfall().expect("欠けている");
        assert!(shortfall.contains("example.com/far"), "{shortfall}");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_old_copy_is_not_missing() {
        // 更新を確かめられなかっただけなら、前の版で引ける。欠けてはいない。
        let directory = scratch("old-copy");
        let mut library = Library::new(directory.join("cache"), pretend);
        let sources = vec![Source::Url("https://example.com/old".to_owned())];
        library.configure(&sources, &directory);
        library.settle();
        assert_eq!(library.shortfall(), None);

        library.fetch = refuse;
        library.checked_updates = false;
        library.loaded = None;
        library.configure(&sources, &directory);
        library.settle();
        assert!(!library.problems().is_empty(), "取得できなかったことは残る");
        assert_eq!(library.shortfall(), None);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_dictionary_is_fetched_again_later() {
        let directory = scratch("retry");
        let mut library = Library::new(directory.join("cache"), refuse);
        let sources = vec![Source::Url("https://example.com/later".to_owned())];
        library.configure(&sources, &directory);
        library.settle();
        assert!(library.shortfall().is_some());

        // 通信できるようになった。次に設定を尋ねられたとき (試験では間を
        // 置かない) に取り直す。
        library.fetch = pretend;
        library.configure(&sources, &directory);
        library.settle();
        assert_eq!(library.shortfall(), None);
        assert!(!library.lookup(&Query::okuri_nashi("とりよせ")).is_empty());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn katakana_words_are_found_by_their_own_reading() {
        // ヴァイオリン は violin の下にしか無い。派生した辞書なら読みで引ける。
        let directory = scratch("katakana");
        fs::write(
            directory.join("l.dict"),
            "violin /ヴァイオリン/バイオリン/\nぱそこん /パソコン/\n",
        )
        .unwrap();
        let base = Source::File(directory.join("l.dict").display().to_string());
        let mut library = Library::new(directory.join("cache"), refuse);
        library.configure(
            &[base.clone(), Source::Katakana(Box::new(base))],
            &directory,
        );
        library.settle();

        assert_eq!(
            words(&library.lookup(&Query::okuri_nashi("う゛ぁいおりん"))),
            ["ヴァイオリン"]
        );
        // 元の辞書にもある語は、一度だけ出る。
        assert_eq!(
            words(&library.lookup(&Query::okuri_nashi("ぱそこん"))),
            ["パソコン"]
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_dictionary_and_its_katakana_words_are_fetched_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn counting(url: &str, path: &Path) -> Result<bool, String> {
            CALLS.fetch_add(1, Ordering::SeqCst);
            pretend(url, path)
        }

        let directory = scratch("fetched-once");
        let mut library = Library::new(directory.join("cache"), counting);
        let l = Source::Url("https://example.com/l".to_owned());
        library.configure(&[l.clone(), Source::Katakana(Box::new(l))], &directory);
        library.settle();
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn updates_are_checked_once_per_start() {
        // 入力先を切り替えるたびに通信はしない。
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn counting(url: &str, path: &Path) -> Result<bool, String> {
            CALLS.fetch_add(1, Ordering::SeqCst);
            pretend(url, path)
        }

        let directory = scratch("updates");
        let mut library = Library::new(directory.join("cache"), counting);
        let sources = [Source::Url("https://example.com/a".to_owned())];
        library.configure(&sources, &directory);
        library.settle();
        library.configure(&[], &directory);
        library.settle();
        library.configure(&sources, &directory);
        library.settle();
        assert_eq!(
            CALLS.load(Ordering::SeqCst),
            1,
            "手元にあれば二度目は取らない"
        );
        let _ = fs::remove_dir_all(&directory);
    }
}
