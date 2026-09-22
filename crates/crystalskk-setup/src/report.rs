//! 伝えたいことの行き先。
//!
//! 昇格して呼び直された側は別のコンソールで動くので、そのまま書いても
//! 誰も読めない。書き出す先を渡されていればそちらへ溜め、渡されて
//! いなければ普通に表示する。

use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// 伝えたいことの行き先。
#[derive(Debug)]
pub struct Report {
    /// 書き出す先。`None` なら標準出力へ。
    path: Option<PathBuf>,
}

impl Report {
    pub fn new(path: Option<&Path>) -> Self {
        Self {
            path: path.map(Path::to_path_buf),
        }
    }

    /// 自分のコンソールへ直に出せるか。
    ///
    /// 昇格して呼ばれた側は別のコンソールなので偽。「本人として確かめ
    /// 直す」処理を、親のときだけ動かすのに使う。
    pub fn is_console(&self) -> bool {
        self.path.is_none()
    }

    /// 一言伝える。改行は呼ぶ側が付ける。
    ///
    /// 書き出しに失敗しても何もしない。伝えられないこと自体は、
    /// 導入の成否を変えないため。
    pub fn say(&self, message: &str) {
        match &self.path {
            Some(path) => {
                let _ = append(path, message);
            }
            None => print!("{message}"),
        }
    }
}

fn append(path: &Path, message: &str) -> io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(message.as_bytes())
}

/// 昇格した側に書かせる一時ファイルの場所。
pub fn temporary_path() -> io::Result<PathBuf> {
    let directory = std::env::temp_dir();
    // 同時に走ることは考えにくいが、残骸と混ざらないよう識別子を付ける。
    let name = format!("crystalskk-setup-{}.txt", std::process::id());
    Ok(directory.join(name))
}

/// 書かれたものを読み出して片付ける。
pub fn take(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_to_a_file_when_given_one() {
        let path = std::env::temp_dir().join("crystalskk-report-test.txt");
        let _ = std::fs::remove_file(&path);

        let report = Report::new(Some(&path));
        report.say("一つ目\n");
        report.say("二つ目\n");

        assert_eq!(take(&path).as_deref(), Some("一つ目\n二つ目\n"));
        assert!(!path.exists(), "読んだら片付ける");
    }

    #[test]
    fn reading_a_missing_report_gives_nothing() {
        let path = std::env::temp_dir().join("crystalskk-report-missing.txt");
        let _ = std::fs::remove_file(&path);
        assert!(take(&path).is_none());
    }
}
