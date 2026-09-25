//! エンジンと辞書を組み合わせた入力セッション。
//!
//! 行入力モードと対話モードで共有する。どちらも「キーを送る」「今の状態を
//! 見る」しか行わない。

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crystalskk_core::dict::{Candidate, CandidateSource, ChainedSource, Query};
use crystalskk_core::engine::{CandidateView, CompletionView, Event, Role};
use crystalskk_core::{Engine, InputMode, Key};
use crystalskk_dict::{MemoryDict, UserDict, encoding};
use crystalskk_settings::Settings;

/// ユーザー辞書を、引きながら書き換えられる形にしたもの。
///
/// エンジンは候補ソースを所有するが、学習ではそれを書き換える必要がある。
/// エンジンに書き換えの口を持たせるより、共有の持ち手をこちら側で用意する
/// ほうが、エンジンを純粋に保てる。
#[derive(Clone)]
struct SharedUserDict(Rc<RefCell<UserDict>>);

impl std::fmt::Debug for SharedUserDict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedUserDict").finish_non_exhaustive()
    }
}

impl CandidateSource for SharedUserDict {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.0.borrow().lookup(query)
    }

    /// 補完も中継する。**書かないと既定の「何も返さない」になり、
    /// ユーザー辞書の語が補完に出てこない。**
    fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        self.0.borrow().complete(prefix, limit)
    }
}

/// 入力セッション。
pub struct Session {
    engine: Engine,
    /// 設定ファイルから読んだ値。エンジンにも渡してある。
    settings: Settings,
    user: Rc<RefCell<UserDict>>,
    /// これまでに確定した文字列。入力先アプリの中身に相当する。
    document: String,
    /// 直近の打鍵がエンジンに処理されなかったか。
    last_unhandled: Option<Key>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("document", &self.document)
            .finish_non_exhaustive()
    }
}

/// セッションを組み立てる。
#[derive(Debug, Default)]
pub struct SessionBuilder {
    dictionaries: Vec<PathBuf>,
    user_dictionary: Option<PathBuf>,
    settings: Option<PathBuf>,
}

impl SessionBuilder {
    pub fn dictionary(mut self, path: impl Into<PathBuf>) -> Self {
        self.dictionaries.push(path.into());
        self
    }

    pub fn user_dictionary(mut self, path: impl Into<PathBuf>) -> Self {
        self.user_dictionary = Some(path.into());
        self
    }

    pub fn settings(mut self, path: impl Into<PathBuf>) -> Self {
        self.settings = Some(path.into());
        self
    }

    /// 辞書を読み込んでセッションを作る。読み込みの経過は `log` へ渡す。
    pub fn build(self, log: &mut dyn FnMut(&str)) -> io::Result<Session> {
        // 設定を先に読む。**既定値では動かない** (ADR-0020)。ファイルが
        // 無ければ雛形から作り、足りなければ書き足す。TIP と同じ読み方で、
        // 置き場所だけが違う。
        let settings_path = self
            .settings
            .unwrap_or_else(|| PathBuf::from(DEFAULT_SETTINGS));
        let loaded = crystalskk_settings::load(&settings_path)
            .map_err(|e| io::Error::other(e.to_string()))?;
        if loaded.created {
            log(&format!("設定 {} を作りました", settings_path.display()));
        } else {
            log(&format!("設定 {}", settings_path.display()));
        }
        if !loaded.added.is_empty() {
            log(&format!("  書き足した項目: {}", loaded.added.join(", ")));
        }
        if !loaded.unknown.is_empty() {
            log(&format!("  知らない項目: {}", loaded.unknown.join(", ")));
        }
        if loaded.romaji_created {
            log(&format!(
                "ローマ字テーブル {} を作りました",
                loaded.romaji_path.display()
            ));
        }

        let user_path = self
            .user_dictionary
            .unwrap_or_else(|| PathBuf::from(DEFAULT_USER_DICTIONARY));
        let (user, report) = UserDict::load(&user_path)?;
        log(&format!(
            "ユーザー辞書 {} ({} 件)",
            user_path.display(),
            report.entries
        ));
        let user = Rc::new(RefCell::new(user));

        let mut sources: Vec<Box<dyn CandidateSource>> =
            vec![Box::new(SharedUserDict(Rc::clone(&user)))];
        for path in &self.dictionaries {
            let dict = load_dictionary(path, log)?;
            sources.push(Box::new(dict));
        }

        let mut engine = Engine::new(Box::new(ChainedSource::new(sources)));
        engine.configure(loaded.settings.engine.clone());

        Ok(Session {
            engine,
            settings: loaded.settings,
            user,
            document: String::new(),
            last_unhandled: None,
        })
    }
}

fn load_dictionary(path: &Path, log: &mut dyn FnMut(&str)) -> io::Result<MemoryDict> {
    let bytes = std::fs::read(path)?;
    let decoded = encoding::decode(&bytes);
    let (dict, report) = MemoryDict::parse(&decoded.text);
    let mut note = format!(
        "辞書 {} ({} 件, {})",
        path.display(),
        report.entries,
        decoded.encoding
    );
    if report.skipped > 0 {
        note.push_str(&format!(", 読み飛ばし {} 行", report.skipped));
    }
    if report.merged > 0 {
        note.push_str(&format!(", 併合 {} 行", report.merged));
    }
    log(&note);
    Ok(dict)
}

impl Session {
    /// 打鍵を一つ送る。
    pub fn press(&mut self, key: Key) {
        let response = self.engine.press(key);
        self.document.push_str(&response.commit);
        self.last_unhandled = (!response.handled).then_some(key);

        for event in response.events {
            match event {
                Event::Learn { query, word } | Event::Register { query, word } => {
                    self.user.borrow_mut().learn(&query, &word);
                }
                Event::Purge { query, word } => {
                    self.user.borrow_mut().purge(&query, &word);
                }
            }
        }

        // エンジンが受け取らなかった打鍵は、そのままアプリに届く。ここでは
        // 入力先アプリの役をこちらが務める。半角英数モードで文字が消えて
        // 見えないのは、この肩代わりがないと起きる。
        if !response.handled {
            self.apply_to_document(key);
        }
    }

    /// エンジンが処理しなかった打鍵を、入力先の文字列に反映する。
    ///
    /// 実際の IME では入力先アプリが行うこと。CLI には入力先がないので
    /// ここで真似る。
    fn apply_to_document(&mut self, key: Key) {
        match key {
            Key::Char(c) => self.document.push(c),
            Key::Space => self.document.push(' '),
            Key::Enter => self.document.push('\n'),
            Key::Tab => self.document.push('\t'),
            Key::Backspace => {
                self.document.pop();
            }
            Key::Escape | Key::Up | Key::Down | Key::Ctrl(_) | Key::Paste => {}
        }
    }

    pub fn press_all(&mut self, keys: impl IntoIterator<Item = Key>) {
        for key in keys {
            self.press(key);
        }
    }

    /// 入力先の中身に相当する文字列。
    pub fn document(&self) -> &str {
        &self.document
    }

    pub fn clear_document(&mut self) {
        self.document.clear();
    }

    /// 入力の途中経過を捨てる。文書は消さない。
    pub fn reset_input(&mut self) {
        self.engine.reset();
        self.last_unhandled = None;
    }

    pub fn mode(&self) -> InputMode {
        self.engine.mode()
    }

    /// 印を含めた未確定表示。
    ///
    /// 動的補完の候補は角括弧で囲む。**ターミナルに下線を引けない**ので、
    /// 打った文字との違いを文字で示すしかない。TIP は線の有無で示す。
    pub fn preedit(&self) -> String {
        self.engine
            .preedit()
            .segments
            .iter()
            .map(|segment| match segment.role {
                Role::Completion => format!("[{}]", segment.text),
                _ => segment.text.clone(),
            })
            .collect()
    }

    /// 消してよいか尋ねている候補。
    pub fn purging(&self) -> Option<String> {
        self.engine.purging()
    }

    /// 辞書登録中なら、登録しようとしている見出し。
    pub fn registering(&self) -> Option<String> {
        self.engine.preedit().registering
    }

    pub fn registration_depth(&self) -> usize {
        self.engine.registration_depth()
    }

    /// いま選んでいる補完候補。
    pub fn completion(&self) -> Option<CompletionView> {
        self.engine.completion()
    }

    /// 設定ファイルから読んだ値。
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn candidates(&self) -> Option<CandidateView> {
        self.engine.candidates()
    }

    pub fn last_unhandled(&self) -> Option<Key> {
        self.last_unhandled
    }

    pub fn user_dictionary_path(&self) -> PathBuf {
        self.user.borrow().path().to_path_buf()
    }

    /// ユーザー辞書を保存する。変更がなければ何もしない。
    pub fn save_user_dictionary(&self) -> io::Result<bool> {
        let mut user = self.user.borrow_mut();
        if !user.is_dirty() {
            return Ok(false);
        }
        user.save()?;
        Ok(true)
    }
}

/// 保存先を指定しなかったときのユーザー辞書。
pub const DEFAULT_USER_DICTIONARY: &str = "crystalskk-user.dict";

/// 場所を指定しなかったときの設定ファイル。
///
/// 置き場所の既定であって、**設定の値の既定ではない**。値は必ずこの
/// ファイルに書かれたものを使う。
pub const DEFAULT_SETTINGS: &str = "crystalskk-config.toml";
