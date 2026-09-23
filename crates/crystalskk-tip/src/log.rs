//! 診断用の記録。
//!
//! TIP は他人のプロセスの中で動くので、標準出力もデバッガも当てにできない。
//! 何が起きているかを知る手段が要る。
//!
//! 記録先は `%LOCALAPPDATA%\CrystalSKK\tip.log`。利用者ごとの場所なので、
//! 機械全体に導入されていても書き込みに権限は要らない。
//!
//! # 詳しさは選べる
//!
//! 一行書くたびにファイルを開く。打鍵のたびにそれをやれば**入力が目に
//! 見えて重くなる** (PRD N-01)。かといって何も出さなければ、実機でしか
//! 起きない不具合は追えない。
//!
//! そこで段階を分ける。**全部出すか何も出さないかの二択にしない。**
//!
//! | 段階 | 出るもの | 一打鍵あたり |
//! |---|---|---|
//! | [`Level::Off`] | 何も出ない (既定) | 書かない |
//! | [`Level::Error`] | 失敗だけ | 普段は書かない |
//! | [`Level::Info`] | 有効化、入切、辞書の読み込み | 普段は書かない |
//! | [`Level::Trace`] | 打鍵ごとの一部始終 | **毎回書く** |
//!
//! `Info` までは「めったに起きないこと」だけなので、常用しても入力の速さに
//! 響かない。打鍵ごとの記録が要るときだけ `Trace` へ上げる。
//!
//! 常用する以上は際限なく伸びては困るので、記録を始めるときに大きさを見て、
//! 育ちすぎていれば一代だけ退ける。
//!
//! # 「どのアプリの、いつの話か」を残す
//!
//! 一つのファイルに、あらゆるアプリの記録が混ざって流れ込む。番号だけでは
//! **どのアプリの記録か分からない**。番号は使い回されるうえ、後から調べよう
//! にもそのプロセスはもう居ない。
//!
//! そこで、記録を始めるときに一度だけ**実行ファイルの名前**を書き、以降の
//! 各行には**時刻**を添える。「この打鍵はあのアプリのものか」「今の操作で
//! 増えた行はどれか」が、これで言い当てられる。
//!
//! # 包装されたアプリでは、環境変数も置き場所も当てにならない
//!
//! ストアの仕組みで包装されたアプリ (MSIX) は隔離された入れ物の中で動く。
//! そこでは二つのことが崩れる。
//!
//! - **環境変数が届かない。** 包装されたアプリは起動の道筋が違うので、
//!   `setx` で設定した値を受け取るとは限らない
//! - **書き込み先がすり替わる。** `%LOCALAPPDATA%` は入れ物ごとの場所へ
//!   向けられ、記録は
//!   `%LOCALAPPDATA%\Packages\<包装の名前>\AC\CrystalSKK\tip.log`
//!   に落ちる
//!
//! 前者は致命的で、**記録が無いことが「読み込まれていない」証しにならなく
//! なる**。診断の道具としては使い物にならない。
//!
//! そこで、環境変数に加えて**目印のファイル**でも段階を指定できる。置き場所は
//! DLL の隣で、隔離された入れ物からも読める。中身に段階の名前を書く。

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::SystemInformation::GetLocalTime;

/// どこまで記録するか。
///
/// 並び順がそのまま詳しさの順になっている。ある段階を選ぶと、それより
/// 上の (数の小さい) ものも出る。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Level {
    /// 何も記録しない。
    #[default]
    Off,
    /// 失敗だけ。
    Error,
    /// 節目の出来事。めったに起きないことに限る。
    Info,
    /// 打鍵ごとの一部始終。**入力が重くなる。**
    Trace,
}

impl Level {
    /// 設定に書かれた名前から読み取る。
    ///
    /// 読み取れない値は [`Level::Trace`] とみなす。記録を頼んだ人が、
    /// 綴りを外したせいで**何も出ないまま待たされる**よりはよい。
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "" | "off" | "0" | "none" => Self::Off,
            "error" => Self::Error,
            "info" | "on" => Self::Info,
            _ => Self::Trace,
        }
    }

    /// 設定に書く名前。
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Info => "info",
            Self::Trace => "trace",
        }
    }
}

/// 目印のファイルの名前。中身に段階の名前を書く。
pub const MARKER_NAME: &str = "log.on";

/// 記録する段階と行き先。起動時に一度だけ決める。
static DESTINATION: OnceLock<Option<(Level, PathBuf)>> = OnceLock::new();

/// 失敗を記録する。
pub fn error(message: &str) {
    put(Level::Error, message);
}

/// 節目の出来事を記録する。
///
/// **めったに起きないことに限る。** 打鍵ごとに呼ぶものをここへ入れると、
/// 段階を分けた意味がなくなる。
pub fn write(message: &str) {
    put(Level::Info, message);
}

/// 打鍵ごとの細かい記録。
pub fn trace(message: &str) {
    put(Level::Trace, message);
}

/// いま打鍵ごとの記録を取るか。
///
/// 記録しないと決まっているなら、渡す文字列を組み立てる手間も省きたい。
pub fn tracing() -> bool {
    level() >= Level::Trace
}

/// いまの段階。
fn level() -> Level {
    destination().map_or(Level::Off, |(level, _)| *level)
}

/// 一行書く。失敗しても何もしない。記録できないことで入力を止めない。
fn put(level: Level, message: &str) {
    let Some((wanted, path)) = destination() else {
        return;
    };
    if level > *wanted {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(file, "[{} {}] {message}", now(), std::process::id());
}

/// 記録する段階と行き先を決める。
fn destination() -> Option<&'static (Level, PathBuf)> {
    DESTINATION
        .get_or_init(|| {
            let level = requested();
            if level == Level::Off {
                return None;
            }
            let base = std::env::var_os("LOCALAPPDATA")?;
            let directory = PathBuf::from(base).join("CrystalSKK");
            std::fs::create_dir_all(&directory).ok()?;
            let path = directory.join("tip.log");
            rotate_if_large(&path);
            announce(&path, level);
            Some((level, path))
        })
        .as_ref()
}

/// 頼まれている段階。
///
/// 環境変数と目印のファイルのどちらでもよい。**詳しいほうを採る。**
/// 包装されたアプリには環境変数が届かないので、ファイルという逃げ道が要る。
fn requested() -> Level {
    let by_variable = std::env::var_os("CRYSTALSKK_LOG")
        .map(|value| Level::parse(&value.to_string_lossy()))
        .unwrap_or_default();
    let by_marker = marker()
        .and_then(|path| std::fs::read_to_string(path).ok())
        // 空のファイルは「段階の指定なし」。置いた以上は記録したいはず
        // なので、いちばん詳しいところから始める。
        .map(|text| {
            if text.trim().is_empty() {
                Level::Trace
            } else {
                Level::parse(&text)
            }
        })
        .unwrap_or_default();
    by_variable.max(by_marker)
}

/// 目印のファイルの場所。DLL と同じところに置く。
///
/// 隔離された入れ物の中のアプリからも読めるよう、導入先に置くのが肝心で、
/// 利用者ごとの場所ではいけない。
fn marker() -> Option<PathBuf> {
    let module = crate::registry::module_path(crate::module()).ok()?;
    let directory = PathBuf::from(module).parent()?.to_path_buf();
    Some(directory.join(MARKER_NAME))
}

/// 大きくなりすぎていたら、一代だけ退けて新しく始める。
///
/// `info` は常用してよい段階だが、**際限なく伸びてよいわけではない**。
/// 誰も見ないまま何ヶ月も肥えるのは、利用者の領域に置くものとして筋が悪い。
///
/// 見るのは記録を始めるときだけにする。書き込みのたびに大きさを調べれば、
/// せっかく軽くした意味がなくなる。
fn rotate_if_large(path: &std::path::Path) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.len() <= MAX_BYTES {
        return;
    }
    // 一代だけ残す。二代目は落とす。**古い記録より新しい記録のほうが
    // 役に立つ。**
    let retired = path.with_extension("log.old");
    let _ = std::fs::rename(path, retired);
}

/// これを超えたら退ける。
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// 記録を始めたことを、どのアプリの中かと共に書く。
///
/// [`destination`] の初期化の中から呼ぶ。普通の書き込みを使うと初期化が
/// 入れ子になるので、ここだけは自分でファイルを開く。
fn announce(path: &std::path::Path, level: Level) {
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(
        file,
        "[{} {}] ===== 読み込まれた: {} ({}) =====",
        now(),
        std::process::id(),
        host(),
        level.name()
    );
}

/// この記録を出しているアプリの実行ファイル。
fn host() -> String {
    let mut buffer = [0u16; 260];
    // SAFETY: 引数に `None` を渡すと、いま動いているプロセスの実行ファイルを
    // 指す。書き込み先は手元の配列で、長さもそのまま渡している。
    let length = unsafe { GetModuleFileNameW(None, &mut buffer) } as usize;
    if length == 0 {
        return "(分からない)".to_owned();
    }
    let path = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
    // 全部の道のりは長いので、名前だけにする。どのアプリかはこれで足りる。
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "(分からない)".to_owned())
}

/// いまの時刻。日付は要らない。一度の確認の中で前後が分かればよい。
fn now() -> String {
    // SAFETY: 値を返すだけの呼び出しで、引数も持たない。
    let time = unsafe { GetLocalTime() };
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        time.wHour, time.wMinute, time.wSecond, time.wMilliseconds
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_levels_are_ordered_by_detail() {
        assert!(Level::Off < Level::Error);
        assert!(Level::Error < Level::Info);
        assert!(Level::Info < Level::Trace);
    }

    #[test]
    fn names_survive_a_round_trip() {
        for level in [Level::Off, Level::Error, Level::Info, Level::Trace] {
            assert_eq!(Level::parse(level.name()), level, "{level:?}");
        }
    }

    #[test]
    fn an_empty_value_means_off() {
        // `setx CRYSTALSKK_LOG ""` で切ったつもりの人を、中身の無い変数の
        // せいで記録し続けるのは理不尽である。
        assert_eq!(Level::parse(""), Level::Off);
        assert_eq!(Level::parse("   "), Level::Off);
        assert_eq!(Level::parse("off"), Level::Off);
    }

    #[test]
    fn a_misspelled_level_records_everything() {
        // 記録を頼んだ人が、綴りを外したせいで何も出ないまま待たされる
        // よりはよい。
        assert_eq!(Level::parse("verbose"), Level::Trace);
        assert_eq!(Level::parse("1"), Level::Trace);
    }

    #[test]
    fn the_old_spelling_still_works() {
        // 以前の `log on` は「記録する」の意だった。節目だけで足りる。
        assert_eq!(Level::parse("on"), Level::Info);
    }

    #[test]
    fn case_and_spacing_do_not_matter() {
        assert_eq!(Level::parse(" TRACE\n"), Level::Trace);
        assert_eq!(Level::parse("Info"), Level::Info);
    }
}
