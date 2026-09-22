//! CrystalSKK をこの環境に導入する。
//!
//! ```text
//! crystalskk-setup install [DLL]
//! crystalskk-setup uninstall [--purge]
//! crystalskk-setup status
//! crystalskk-setup log on|off
//! ```
//!
//! 入力方式の登録は機械全体に書かれるため、管理者権限が要る (ADR-0007)。
//! 権限がないときは自分を昇格して呼び直すので、利用者は UAC の確認に
//! 応じるだけでよい。
//!
//! これは配布用のインストーラではない。本物のインストーラを作るときは、
//! この処理をそのまま中身として使う。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crystalskk_tip::com::Apartment;

mod dictionary;
mod elevate;
mod install;
mod report;

use report::Report;

fn main() -> ExitCode {
    // 入力方式の登録は COM 越しに行う。抜けるのは持ち手が面倒を見る。
    let _apartment = Apartment::enter();
    run(std::env::args().skip(1).collect())
}

fn run(arguments: Vec<String>) -> ExitCode {
    let parsed = match Options::parse(&arguments) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => return fail(&message, None),
    };

    match parsed.command {
        Command::Status => return report_status(),
        // 辞書は利用者ごとの場所へ置くので、権限は要らない。
        Command::Dict => return dictionary::fetch(),
        _ => {}
    }

    // 権限が要る操作。足りなければ昇格して同じことをやり直す。
    if !parsed.no_elevate && !elevate::is_elevated() {
        return elevated_pass(&arguments);
    }

    let report = Report::new(parsed.report.as_deref());
    match parsed.command {
        Command::Install => do_install(parsed.dll.as_deref(), &report),
        Command::Uninstall => do_uninstall(parsed.purge, &report),
        Command::LogOn => do_log(true, &report),
        Command::LogOff => do_log(false, &report),
        Command::Status | Command::Dict => unreachable!("上で処理済み"),
    }
}

/// 昇格した自分を呼び、その結果をこちらのコンソールへ出す。
fn elevated_pass(arguments: &[String]) -> ExitCode {
    println!("登録には管理者権限が要ります。確認を求めます。");

    let Ok(report_path) = report::temporary_path() else {
        return fail("一時ファイルを用意できません", None);
    };

    let mut forwarded: Vec<String> = arguments.to_vec();
    forwarded.push("--no-elevate".to_owned());
    forwarded.push("--report".to_owned());
    forwarded.push(report_path.to_string_lossy().into_owned());

    match elevate::relaunch_as_administrator(&forwarded) {
        Ok(code) => {
            if let Some(message) = report::take(&report_path) {
                print!("{message}");
            }
            if code == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => fail(&e.to_string(), None),
    }
}

fn do_install(source: Option<&Path>, report: &Report) -> ExitCode {
    let source = match source.map(PathBuf::from).or_else(default_source) {
        Some(path) => path,
        None => {
            return fail(
                "DLL が見つかりません。場所を渡すか、先に \
                 cargo build -p crystalskk-tip --release を実行してください",
                Some(report),
            );
        }
    };

    match install::install(&source) {
        Ok(installed) => {
            let verb = if installed.replaced {
                "入れ替えました"
            } else {
                "導入しました"
            };
            // どの DLL を入れたかは必ず示す。黙って選ぶと、古いものや
            // debug ビルドが入っていても気づけない。
            report.say(&format!("元: {}\n", installed.source.display()));
            report.say(&format!("先: {}\n", installed.dll.display()));
            report.say(&format!("CrystalSKK を{verb}。\n"));
            if is_debug_build(&installed.source) {
                report.say("\n");
                report.say("これは debug ビルドです。動きはしますが遅い。\n");
                report.say("cargo build -p crystalskk-tip --release を先に実行してください。\n");
            }
            if installed.retired {
                report.say("\n");
                report.say("古い DLL は使用中だったので、別名へ退けました。\n");
                report.say("すでに開いているアプリは古いほうを使い続けます。\n");
                report.say("入れ替えを反映するには、そのアプリを開き直してください。\n");
            }
            if installed.cleared_per_user {
                report.say("\n");
                report.say("古い利用者ごとの登録を消しました。\n");
                report.say("そちらが優先されるため、残っていると古い DLL が使われます。\n");
            }
            report.say("\n");
            report.say("設定 → 時刻と言語 → 言語と地域 → 日本語 → 言語のオプション →\n");
            report.say("キーボード に CrystalSKK が現れます。\n");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.to_string(), Some(report)),
    }
}

fn do_uninstall(purge: bool, report: &Report) -> ExitCode {
    match install::uninstall(purge) {
        Ok(()) => {
            report.say("CrystalSKK の登録を解除しました。\n");
            if purge {
                report.say("写した DLL も削除しました。\n");
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.to_string(), Some(report)),
    }
}

/// 記録の目印を置く、あるいは外す。
///
/// 環境変数ではなくファイルにするのは、**包装されたアプリに環境変数が
/// 届かない**ため。目印は DLL の隣に置く。そこなら隔離された入れ物の中
/// からも読める。
fn do_log(on: bool, report: &Report) -> ExitCode {
    let directory = match install::install_dir() {
        Ok(directory) => directory,
        Err(e) => return fail(&e.to_string(), Some(report)),
    };
    let marker = directory.join(crystalskk_tip::log::MARKER_NAME);

    let result = if on {
        std::fs::write(&marker, b"")
    } else {
        match std::fs::remove_file(&marker) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    };
    if let Err(e) = result {
        return fail(&e.to_string(), Some(report));
    }

    if on {
        report.say("記録を始めます。アプリを開き直すと効きます。\n");
        report.say("\n");
        report.say(concat!(r"記録先: %LOCALAPPDATA%\CrystalSKK\tip.log", "\n"));
        report.say("包装されたアプリ (MSIX) では、そこではなく\n");
        report.say(concat!(
            r"%LOCALAPPDATA%\Packages\<包装の名前>\AC\CrystalSKK\tip.log",
            " に落ちます。\n"
        ));
        report.say("\n");
        report.say("記録は入力のたびに書かれるので、確認が済んだら log off してください。\n");
    } else {
        report.say("記録をやめます。アプリを開き直すと効きます。\n");
    }
    ExitCode::SUCCESS
}

fn report_status() -> ExitCode {
    let status = install::status();
    if !status.is_installed() {
        println!("導入されていません。");
        return ExitCode::SUCCESS;
    }

    if let Some(machine) = &status.machine {
        println!("機械全体:     {}", machine.display());
    }
    if let Some(per_user) = &status.per_user {
        println!("利用者ごと:   {}", per_user.display());
    }

    let Some(effective) = status.effective() else {
        return ExitCode::SUCCESS;
    };
    println!("実際に使う:   {}", effective.display());

    if !effective.is_file() {
        println!();
        println!("登録されている場所に DLL がありません。");
        println!("install し直してください。");
    }
    if status.per_user.is_some() && status.machine.is_some() {
        println!();
        println!("利用者ごとの登録が機械全体の登録より優先されています。");
        println!("install し直すと、古いほうは消えます。");
    }
    ExitCode::SUCCESS
}

/// 見るからに debug ビルドの置き場所か。
fn is_debug_build(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("debug"))
}

/// 場所を指定されなかったときに探す先。
///
/// release を実行ファイルの隣より先に見る。`cargo run` で呼ばれると隣は
/// `target/debug` になり、直前に release を作っていても debug が選ばれて
/// しまうため。配って使うときは `target` が無いので、隣が選ばれる。
fn default_source() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("target/release").join(install::DLL_NAME)];
    if let Ok(exe) = std::env::current_exe()
        && let Some(directory) = exe.parent()
    {
        candidates.push(directory.join(install::DLL_NAME));
    }
    candidates.push(PathBuf::from("target/debug").join(install::DLL_NAME));
    candidates.into_iter().find(|path| path.is_file())
}

fn fail(message: &str, report: Option<&Report>) -> ExitCode {
    match report {
        Some(report) => report.say(&format!("crystalskk-setup: {message}\n")),
        None => eprintln!("crystalskk-setup: {message}"),
    }
    ExitCode::FAILURE
}

/// 何をするか。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Command {
    Install,
    Uninstall,
    /// 辞書を取得して置く。
    Dict,
    /// 診断の記録を始める。
    LogOn,
    /// 診断の記録をやめる。
    LogOff,
    /// 何も変えない。命令が決まるまでの置き場所でもある。
    #[default]
    Status,
}

/// 解釈済みの指定。
#[derive(Debug, Default)]
struct Parsed {
    command: Command,
    dll: Option<PathBuf>,
    purge: bool,
    /// 昇格して呼び直さない。昇格した側に渡される。
    no_elevate: bool,
    /// 伝えたいことを書き出す先。昇格した側に渡される。
    report: Option<PathBuf>,
}

/// 起動時の指定を読む係。
struct Options;

impl Options {
    /// `None` は「使い方を表示して終わり」の意。
    fn parse(arguments: &[String]) -> Result<Option<Parsed>, String> {
        let mut command: Option<Command> = None;
        let mut parsed = Parsed::default();
        let mut rest = arguments.iter();

        while let Some(argument) = rest.next() {
            match argument.as_str() {
                "-h" | "--help" | "help" => return Ok(None),
                "install" => set(&mut command, Command::Install)?,
                "uninstall" => set(&mut command, Command::Uninstall)?,
                "status" => set(&mut command, Command::Status)?,
                "dict" => set(&mut command, Command::Dict)?,
                "log" => {
                    let which = rest.next().ok_or("log には on か off が要ります")?;
                    let command_for = match which.as_str() {
                        "on" => Command::LogOn,
                        "off" => Command::LogOff,
                        other => return Err(format!("log に書けるのは on か off です: {other}")),
                    };
                    set(&mut command, command_for)?;
                }
                "--purge" => parsed.purge = true,
                "--no-elevate" => parsed.no_elevate = true,
                "--report" => {
                    let path = rest.next().ok_or("--report に書き出す先が要ります")?;
                    parsed.report = Some(PathBuf::from(path));
                }
                other if other.starts_with('-') => {
                    return Err(format!("知らない指定です: {other}"));
                }
                other => {
                    if parsed.dll.is_some() {
                        return Err(format!("DLL を二つ渡されました: {other}"));
                    }
                    parsed.dll = Some(PathBuf::from(other));
                }
            }
        }

        let Some(command) = command else {
            return Ok(None);
        };
        if parsed.purge && command != Command::Uninstall {
            return Err("--purge は uninstall にだけ使えます".to_owned());
        }
        if parsed.dll.is_some() && command != Command::Install {
            return Err("DLL を渡せるのは install だけです".to_owned());
        }

        parsed.command = command;
        Ok(Some(parsed))
    }
}

fn set(slot: &mut Option<Command>, command: Command) -> Result<(), String> {
    if slot.is_some() {
        return Err("命令は一つだけ書いてください".to_owned());
    }
    *slot = Some(command);
    Ok(())
}

const USAGE: &str = "\
crystalskk-setup - CrystalSKK をこの環境に導入する

使い方:
  crystalskk-setup install [DLL]      導入する (DLL 省略時はビルド成果物を探す)
  crystalskk-setup uninstall          登録を解除する
  crystalskk-setup uninstall --purge  写した DLL も削除する
  crystalskk-setup status             今の状態を表示する
  crystalskk-setup dict               辞書を取得して置く (権限は要らない)
  crystalskk-setup log on|off         診断の記録を始める/やめる

install と uninstall と log には管理者権限が要る。権限がなければ UAC の確認を出して
自分を呼び直すので、確認に応じてほしい。
";

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Option<Parsed>, String> {
        let arguments: Vec<String> = arguments.iter().map(|s| (*s).to_owned()).collect();
        Options::parse(&arguments)
    }

    #[test]
    fn reads_a_bare_command() {
        let parsed = parse(&["install"]).expect("読める").expect("命令がある");
        assert_eq!(parsed.command, Command::Install);
        assert!(parsed.dll.is_none());
        assert!(!parsed.no_elevate);
    }

    #[test]
    fn reads_the_dll_and_the_forwarded_options() {
        let parsed = parse(&["install", "a.dll", "--no-elevate", "--report", "r.txt"])
            .expect("読める")
            .expect("命令がある");
        assert_eq!(parsed.dll, Some(PathBuf::from("a.dll")));
        assert!(parsed.no_elevate);
        assert_eq!(parsed.report, Some(PathBuf::from("r.txt")));
    }

    #[test]
    fn reads_the_log_switch() {
        for (word, expected) in [("on", Command::LogOn), ("off", Command::LogOff)] {
            let parsed = parse(&["log", word]).expect("読める").expect("命令がある");
            assert_eq!(parsed.command, expected);
        }
    }

    #[test]
    fn log_needs_on_or_off() {
        assert!(parse(&["log"]).is_err());
        assert!(parse(&["log", "maybe"]).is_err());
    }

    #[test]
    fn no_command_asks_for_the_usage() {
        assert!(parse(&[]).expect("読める").is_none());
        assert!(parse(&["--help"]).expect("読める").is_none());
    }

    #[test]
    fn rejects_options_on_the_wrong_command() {
        assert!(parse(&["install", "--purge"]).is_err());
        assert!(parse(&["status", "a.dll"]).is_err());
        assert!(parse(&["install", "uninstall"]).is_err());
        assert!(parse(&["install", "a.dll", "b.dll"]).is_err());
        assert!(parse(&["--report"]).is_err());
    }
}
