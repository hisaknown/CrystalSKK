//! 辞書のダウンロードと設置。
//!
//! CrystalSKK は辞書を同梱しない (PRD §10)。利用者の操作で取得し、
//! UTF-8 に変換して保存する (ADR-0003)。
//!
//! 通信は OS の HTTP スタックに任せる (ADR-0004)。TLS の実装も証明書の
//! 検証もプロキシ設定も Windows のものを使うため、この工程のために
//! 暗号ライブラリを抱え込まない。

use std::fs;
use std::io;
use std::path::Path;

use crystalskk_dict::{MemoryDict, encoding};

pub mod url;

#[cfg(windows)]
mod winhttp;

/// skk-dev/dict の L 辞書。SKK 辞書の標準的な選択。
///
/// この辞書は GPL 系のライセンスであり、CrystalSKK (MIT) とは別物である。
/// 同梱せず、利用者の操作で取得する。
pub const SKK_JISYO_L: &str = "https://raw.githubusercontent.com/skk-dev/dict/master/SKK-JISYO.L";

/// 取得に失敗する理由。
#[derive(Debug)]
pub enum Error {
    /// 扱えない URL。
    BadUrl(String),
    /// 200 でも 304 でもない応答。
    Http(u16),
    /// OS から返ってきた失敗。
    Os(String),
    /// 保存に失敗した。
    Io(io::Error),
    /// この環境では取得を行えない。
    Unsupported,
}

impl Error {
    fn bad_url(url: &str) -> Self {
        Self::BadUrl(url.to_owned())
    }

    #[cfg(windows)]
    fn from_last_os_error() -> Self {
        Self::Os(io::Error::last_os_error().to_string())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadUrl(url) => write!(f, "扱えない URL です: {url}"),
            Self::Http(status) => write!(f, "取得に失敗しました (HTTP {status})"),
            Self::Os(message) => write!(f, "通信に失敗しました: {message}"),
            Self::Io(e) => write!(f, "保存に失敗しました: {e}"),
            Self::Unsupported => write!(f, "この環境では辞書を取得できません"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

#[cfg(windows)]
impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        Self::Os(e.message())
    }
}

/// 取得した中身。
#[derive(Debug, Clone)]
pub struct Downloaded {
    pub body: Vec<u8>,
    /// 応答の `ETag`。次回の取得で更新の有無を問い合わせるのに使う。
    pub etag: Option<String>,
}

/// 取得の結果。
#[derive(Debug, Clone)]
pub enum Fetched {
    /// 前回から変わっていない。
    NotModified,
    /// 取得できた。
    Downloaded(Downloaded),
}

/// URL から取得する。`etag` を渡すと、変化がなければ [`Fetched::NotModified`]。
#[cfg(windows)]
pub fn get(url: &str, etag: Option<&str>) -> Result<Fetched, Error> {
    winhttp::get(url, etag)
}

/// Windows 以外では取得を行えない。
///
/// 辞書の設置は Windows 上でしか起きないが、変換と保存の部分は他の環境でも
/// 試験できるようにしておきたいので、クレート自体はビルドできるようにする。
#[cfg(not(windows))]
pub fn get(_url: &str, _etag: Option<&str>) -> Result<Fetched, Error> {
    Err(Error::Unsupported)
}

/// 設置の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    /// 取得元の文字コード。
    pub source_encoding: &'static str,
    /// 読み込めた見出しの数。
    pub entries: usize,
    /// 解釈できずに読み飛ばした行の数。
    pub skipped: usize,
    /// 既出の見出しと併合した行の数。
    pub merged: usize,
    /// 保存したファイルの大きさ。
    pub bytes_written: u64,
    /// 次回の取得に使う `ETag`。
    pub etag: Option<String>,
}

/// 取得したバイト列を UTF-8 の辞書として保存する。
///
/// 通信を伴わないので、取得部分と切り離して試験できる。
pub fn install_bytes(bytes: &[u8], path: &Path) -> Result<InstallReport, Error> {
    let decoded = encoding::decode(bytes);
    let (dict, report) = MemoryDict::parse(&decoded.text);

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = std::path::PathBuf::from(temporary);

    let text = dict.to_skk_text();
    fs::write(&temporary, text.as_bytes())?;
    fs::rename(&temporary, path)?;

    Ok(InstallReport {
        source_encoding: decoded.encoding,
        entries: report.entries,
        skipped: report.skipped,
        merged: report.merged,
        bytes_written: text.len() as u64,
        etag: None,
    })
}

/// 辞書を取得して設置する。前回から変化がなければ `None`。
pub fn install(url: &str, path: &Path, etag: Option<&str>) -> Result<Option<InstallReport>, Error> {
    match get(url, etag)? {
        Fetched::NotModified => Ok(None),
        Fetched::Downloaded(downloaded) => {
            let mut report = install_bytes(&downloaded.body, path)?;
            report.etag = downloaded.etag;
            Ok(Some(report))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crystalskk_core::dict::{CandidateSource, Query};
    use crystalskk_dict::UserDict;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("crystalskk-fetch-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("作業ディレクトリを作れる");
        dir
    }

    #[test]
    fn installs_a_euc_jp_dictionary_as_utf8() {
        let path = scratch("install").join("SKK-JISYO.L");
        let source = ";; -*- coding: euc-jp -*-\n;; okuri-nasi entries.\nかんじ /漢字/感じ/\n";
        let (bytes, _, _) = encoding_rs::EUC_JP.encode(source);

        let report = install_bytes(&bytes, &path).expect("設置できる");
        assert_eq!(report.source_encoding, "EUC-JP");
        assert_eq!(report.entries, 1);
        assert_eq!(report.skipped, 0);

        let written = fs::read(&path).expect("読める");
        assert!(std::str::from_utf8(&written).is_ok(), "保存は UTF-8");

        // 保存したものをそのまま辞書として引ける。
        let (dict, _) = UserDict::load(&path).expect("読み込める");
        let got = dict.lookup(&Query::okuri_nashi("かんじ"));
        assert_eq!(got[0].word, "漢字");
    }

    #[test]
    fn installing_replaces_the_previous_file() {
        let dir = scratch("replace");
        let path = dir.join("SKK-JISYO.L");
        install_bytes("かんじ /漢字/".as_bytes(), &path).expect("設置できる");
        install_bytes("ことば /言葉/".as_bytes(), &path).expect("設置できる");

        let text = fs::read_to_string(&path).expect("読める");
        assert!(text.contains("言葉"));
        assert!(!text.contains("漢字"));
        assert!(
            !dir.join("SKK-JISYO.L.tmp").exists(),
            "一時ファイルは残さない"
        );
    }

    #[test]
    fn broken_lines_are_reported_but_do_not_stop_the_install() {
        let path = scratch("broken").join("SKK-JISYO.L");
        let report = install_bytes("かんじ /漢字/\nこわれた\nことば /言葉/\n".as_bytes(), &path)
            .expect("設置できる");
        assert_eq!(report.entries, 2);
        assert_eq!(report.skipped, 1);
    }
}
