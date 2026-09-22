//! CrystalSKK をターミナルから動かす。
//!
//! 既定は行入力モードで、一行を打鍵列として受け取る。`-i` を付けると
//! 一打鍵ずつ受け取る対話モードになる。

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use crystalskk_cli::session::SessionBuilder;
use crystalskk_cli::{interactive, line};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("crystalskk: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{USAGE}");
            return Ok(());
        }
        Err(message) => return Err(io::Error::other(message)),
    };

    let mut builder = SessionBuilder::default();
    for path in &options.dictionaries {
        builder = builder.dictionary(path);
    }
    if let Some(path) = &options.user_dictionary {
        builder = builder.user_dictionary(path);
    }

    let mut log = |note: &str| eprintln!("  {note}");
    let mut session = builder.build(&mut log)?;
    if options.dictionaries.is_empty() {
        eprintln!("  辞書がありません。変換はすべて辞書登録になります。");
        eprintln!("  cargo run -p crystalskk-fetch --example install-dict -- ./SKK-JISYO.L");
    }
    eprintln!();

    if options.interactive {
        interactive::run(&mut session)?;
    } else {
        line::run(&mut session, io::stdin().is_terminal())?;
    }

    // 学習した内容は黙って捨てない。
    match session.save_user_dictionary() {
        Ok(true) => {
            let mut err = io::stderr();
            writeln!(
                err,
                "ユーザー辞書を保存しました: {}",
                session.user_dictionary_path().display()
            )?;
        }
        Ok(false) => {}
        Err(e) => eprintln!("ユーザー辞書を保存できませんでした: {e}"),
    }
    Ok(())
}

/// 起動時の指定。
#[derive(Debug, Default)]
struct Options {
    dictionaries: Vec<PathBuf>,
    user_dictionary: Option<PathBuf>,
    interactive: bool,
}

impl Options {
    /// `None` は「使い方を表示して終わり」の意。
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => return Ok(None),
                "-i" | "--interactive" => options.interactive = true,
                "--dict" => {
                    let path = args.next().ok_or("--dict に辞書の場所が要ります")?;
                    options.dictionaries.push(PathBuf::from(path));
                }
                "--user-dict" => {
                    let path = args.next().ok_or("--user-dict に辞書の場所が要ります")?;
                    options.user_dictionary = Some(PathBuf::from(path));
                }
                other if other.starts_with('-') => {
                    return Err(format!("知らない指定です: {other}"));
                }
                // 指定のない引数は辞書として扱う。
                other => options.dictionaries.push(PathBuf::from(other)),
            }
        }
        Ok(Some(options))
    }
}

const USAGE: &str = "\
crystalskk - CrystalSKK の変換をターミナルで動かす

使い方:
  crystalskk [指定] [辞書...]

指定:
  --dict <場所>       静的辞書。繰り返し指定できる
  --user-dict <場所>  ユーザー辞書 (既定: crystalskk-user.dict)
  -i, --interactive   一打鍵ずつ受け取る対話モード
  -h, --help          この説明

既定は行入力モード。一行を打鍵列として読む。
  Kanji\\s   →  ▼漢字
表記の詳細は起動後に :help と入力する。
";

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Options>, String> {
        Options::parse(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn bare_arguments_are_dictionaries() {
        let options = parse(&["a.dict", "b.dict"])
            .expect("読める")
            .expect("使い方ではない");
        assert_eq!(
            options.dictionaries,
            [PathBuf::from("a.dict"), PathBuf::from("b.dict")]
        );
        assert!(!options.interactive);
    }

    #[test]
    fn named_options_are_read() {
        let options = parse(&["--dict", "a.dict", "--user-dict", "u.dict", "-i"])
            .expect("読める")
            .expect("使い方ではない");
        assert_eq!(options.dictionaries, [PathBuf::from("a.dict")]);
        assert_eq!(options.user_dictionary, Some(PathBuf::from("u.dict")));
        assert!(options.interactive);
    }

    #[test]
    fn help_asks_for_the_usage() {
        assert!(parse(&["--help"]).expect("読める").is_none());
    }

    #[test]
    fn reports_what_it_cannot_read() {
        assert!(parse(&["--dict"]).is_err());
        assert!(parse(&["--nope"]).is_err());
    }
}
