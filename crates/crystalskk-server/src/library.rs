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

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;

use crystalskk_core::dict::{Candidate, CandidateSource, Query};
use crystalskk_dict::{MemoryDict, encoding};
use crystalskk_settings::Source;

/// URL から取ってきて、手元の場所に置く。変わっていれば真。
///
/// 試験では差し替える。**試験がネットワークに出てはいけない。**
pub type Fetch = fn(url: &str, path: &Path) -> Result<bool, String>;

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
    /// 読んだときのファイルの時刻。変わっていれば読み直す。
    stamp: Option<SystemTime>,
}

impl Entry {
    /// 時刻を除いた、どこから読むかだけの形。
    fn place(&self) -> (&str, &Path, Option<&str>) {
        (&self.written, &self.path, self.url.as_deref())
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
    fn locate(&self, source: &Source, settings_directory: &Path) -> Result<Entry, String> {
        let written = source.as_written().to_owned();
        let (path, url) = match source {
            Source::Url(url) => {
                let path = crystalskk_fetch::cache_path(&self.cache, url)
                    .map_err(|e| format!("{written}: {e}"))?;
                (path, Some(url.clone()))
            }
            Source::File(_) => {
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

    for entry in &mut entries {
        if let Some(url) = &entry.url
            && (check_updates || !entry.path.exists())
        {
            match fetch(url, &entry.path) {
                Ok(true) => eprintln!("crystalskk-server: 辞書を取得しました: {url}"),
                Ok(false) => {}
                // 手元に前のものがあれば、それで続ける。通信できないのは
                // よくあることで、辞書が引けなくなるほどのことではない。
                Err(e) => problems.push(format!("{} を取得できません: {e}", entry.written)),
            }
        }

        match fs::read(&entry.path) {
            Ok(bytes) => {
                let decoded = encoding::decode(&bytes);
                let (dict, _) = MemoryDict::parse(&decoded.text);
                shelves.push(Shelf {
                    name: entry.written.clone(),
                    dict,
                });
            }
            Err(e) => problems.push(format!("{} を読めません: {e}", entry.written)),
        }
        entry.stamp = modified(&entry.path);
    }

    Outcome {
        generation: 0,
        shelves,
        entries,
        problems,
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
