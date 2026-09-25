//! ユーザー辞書。
//!
//! 学習結果と登録語を持つ、書き換えられる唯一の辞書。他の SKK 実装と
//! 行き来できることを重視し、保存形式は SKK 標準のテキストのままにする
//! (PRD N-10)。読み込みは EUC-JP も受けるが、書き出しは常に UTF-8
//! (ADR-0003)。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crystalskk_core::dict::{Candidate, CandidateSource, Query};

use crate::encoding;
use crate::memory::{LoadReport, MemoryDict};

/// ユーザー辞書。
#[derive(Debug, Clone)]
pub struct UserDict {
    dict: MemoryDict,
    path: PathBuf,
    dirty: bool,
}

impl UserDict {
    /// 空のユーザー辞書を作る。保存先だけ決めておく。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            dict: MemoryDict::new(),
            path: path.into(),
            dirty: false,
        }
    }

    /// ファイルから読み込む。ファイルがなければ空の辞書として始める。
    ///
    /// 初回起動時にファイルが存在しないのは異常ではないので、エラーにしない。
    pub fn load(path: impl Into<PathBuf>) -> io::Result<(Self, LoadReport)> {
        let path = path.into();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok((Self::new(path), LoadReport::default()));
            }
            Err(e) => return Err(e),
        };
        let decoded = encoding::decode(&bytes);
        // **並び順を覚えて読む。** ユーザー辞書は使った順に並んでおり、
        // 書き戻すときも同じ順に戻す。崩すと、他の SKK から持ち込んだ
        // 辞書の順序が失われる。
        let (dict, report) = MemoryDict::parse_ordered(&decoded.text);
        Ok((
            Self {
                dict,
                path,
                dirty: false,
            },
            report,
        ))
    }

    /// 保存先。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 前回の保存以降に変更があったか。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 読み出し用の内部辞書。補完などに使う。
    pub fn dict(&self) -> &MemoryDict {
        &self.dict
    }

    /// 確定した語を学習する。辞書登録も同じ操作になる。
    pub fn learn(&mut self, query: &Query, word: &str) {
        self.dict.learn(query, word);
        self.dirty = true;
    }

    /// 候補を一つ消す。消すものがなければ `false`。
    pub fn purge(&mut self, query: &Query, word: &str) -> bool {
        let purged = self.dict.purge(query, word);
        self.dirty |= purged;
        purged
    }

    /// 見出しを削除する。消すものがなければ `false`。
    pub fn remove(&mut self, key: &str, okuri_ari: bool) -> bool {
        let removed = self.dict.remove(key, okuri_ari);
        self.dirty |= removed;
        removed
    }

    /// ファイルへ書き出す。変更がなければ何もしない。
    ///
    /// 一時ファイルへ書いてから差し替える。書き込み中に電源が落ちても、
    /// 既存のユーザー辞書は元のまま残る。
    pub fn save(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        self.save_to(&self.path.clone())?;
        self.dirty = false;
        Ok(())
    }

    /// 変更の有無に関わらず、指定した場所へ書き出す。
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }

        // 拡張子を置き換えるのではなく足す。`a.dict` と `a.txt` が同じ
        // 一時ファイル名を取り合うのを避けるため。
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        {
            use io::Write as _;
            let mut file = fs::File::create(&temporary)?;
            file.write_all(self.dict.to_skk_text().as_bytes())?;
            // 差し替える前に、中身が確実に書かれていることを保証する。
            file.sync_all()?;
        }
        fs::rename(&temporary, path)
    }
}

impl CandidateSource for UserDict {
    fn lookup(&self, query: &Query) -> Vec<Candidate> {
        self.dict.lookup(query)
    }

    fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        CandidateSource::complete(&self.dict, prefix, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 試験ごとに固有の作業ディレクトリ。
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crystalskk-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("作業ディレクトリを作れる");
        dir
    }

    #[test]
    fn missing_file_starts_an_empty_dictionary() {
        let path = scratch("missing").join("user.dict");
        let (dict, report) = UserDict::load(&path).expect("読み込める");
        assert!(dict.dict().is_empty());
        assert_eq!(report, LoadReport::default());
        assert!(!dict.is_dirty());
    }

    #[test]
    fn learning_marks_the_dictionary_dirty() {
        let path = scratch("dirty").join("user.dict");
        let mut dict = UserDict::new(&path);
        assert!(!dict.is_dirty());
        dict.learn(&Query::okuri_nashi("かんじ"), "漢字");
        assert!(dict.is_dirty());
    }

    #[test]
    fn saves_and_loads_back() {
        let path = scratch("roundtrip").join("user.dict");
        let mut dict = UserDict::new(&path);
        dict.learn(&Query::okuri_nashi("かんじ"), "漢字");
        dict.learn(&Query::okuri_ari("おく", 'r', "り"), "送");
        dict.save().expect("保存できる");
        assert!(!dict.is_dirty());

        let (reloaded, report) = UserDict::load(&path).expect("読み込める");
        assert_eq!(report.entries, 2);
        assert_eq!(
            reloaded.lookup(&Query::okuri_nashi("かんじ"))[0].word,
            "漢字"
        );
        assert_eq!(
            reloaded.lookup(&Query::okuri_ari("おく", 'r', "り"))[0].word,
            "送"
        );
    }

    #[test]
    fn saves_as_utf8_even_when_loaded_from_euc_jp() {
        let dir = scratch("euc");
        let path = dir.join("user.dict");
        let source = ";; -*- coding: euc-jp -*-\n;; okuri-nasi entries.\nかんじ /漢字/\n";
        let (bytes, _, _) = encoding_rs::EUC_JP.encode(source);
        fs::write(&path, &bytes).expect("書ける");

        let (mut dict, report) = UserDict::load(&path).expect("読み込める");
        assert_eq!(report.entries, 1);

        dict.learn(&Query::okuri_nashi("ことば"), "言葉");
        dict.save().expect("保存できる");

        let written = fs::read(&path).expect("読める");
        assert!(std::str::from_utf8(&written).is_ok(), "保存は常に UTF-8");
        assert!(String::from_utf8_lossy(&written).contains("coding: utf-8"));
    }

    #[test]
    fn saving_without_changes_does_nothing() {
        let path = scratch("clean").join("user.dict");
        let mut dict = UserDict::new(&path);
        dict.save().expect("何もしないが成功する");
        assert!(!path.exists(), "変更がなければファイルも作らない");
    }

    #[test]
    fn a_failed_write_leaves_the_previous_file_intact() {
        let dir = scratch("atomic");
        let path = dir.join("user.dict");
        let mut dict = UserDict::new(&path);
        dict.learn(&Query::okuri_nashi("かんじ"), "漢字");
        dict.save().expect("保存できる");
        let first = fs::read_to_string(&path).expect("読める");

        // 差し替えは rename で行うため、書き出し途中の内容が本体に現れることはない。
        dict.learn(&Query::okuri_nashi("ことば"), "言葉");
        dict.save().expect("保存できる");
        let second = fs::read_to_string(&path).expect("読める");

        assert!(first.contains("漢字"));
        assert!(second.contains("漢字") && second.contains("言葉"));
        assert!(
            !dir.join("user.dict.tmp").exists(),
            "一時ファイルは残さない"
        );
    }

    #[test]
    fn creates_the_parent_directory() {
        let path = scratch("mkdir")
            .join("nested")
            .join("deeper")
            .join("user.dict");
        let mut dict = UserDict::new(&path);
        dict.learn(&Query::okuri_nashi("かんじ"), "漢字");
        dict.save().expect("親ディレクトリごと作られる");
        assert!(path.exists());
    }
}
