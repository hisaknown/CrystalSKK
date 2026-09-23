//! CrystalSKK をこの環境に導入する。
//!
//! ```text
//! crystalskk-setup install [DLL]
//! crystalskk-setup uninstall [--purge]
//! crystalskk-setup status
//! crystalskk-setup log off|error|info|trace
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
use crystalskk_tip::log::Level;

mod access;
mod dictionary;
mod elevate;
mod install;
mod ranker;
mod report;
mod server;
mod settings;

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

    // 利用者ごとの登録は、**昇格する前に**消す。昇格した側の
    // `HKEY_CURRENT_USER` は、この利用者のものとは限らない。別の管理者の
    // 資格情報で昇格すれば、そちらの利用者の登録を消して終わってしまう。
    //
    // ここは必ずログオンしている本人として動く。消すならここである。
    if matches!(parsed.command, Command::Install | Command::Uninstall) {
        clear_per_user_registration();
    }
    // 設定ファイルも利用者ごとの場所にある。同じ理由で、昇格する前に。
    // 昇格して呼び直された側 (報告の置き場所を渡されている) ではやらない。
    if parsed.command == Command::Install && parsed.report.is_none() {
        settings::prepare();
    }

    // 権限が要る操作。足りなければ昇格して同じことをやり直す。
    if !parsed.no_elevate && !elevate::is_elevated() {
        return elevated_pass(&arguments);
    }

    let report = Report::new(parsed.report.as_deref());
    match parsed.command {
        Command::Install => do_install(
            parsed.dll.as_deref(),
            parsed.ranker_from.as_deref(),
            &report,
        ),
        Command::Uninstall => do_uninstall(parsed.purge, &report),
        Command::Log(level) => do_log(level, &report),
        Command::Status | Command::Dict => unreachable!("上で処理済み"),
    }
}

/// 利用者ごとの COM 登録を消す。
///
/// 残っていると**そちらが機械全体の登録より優先される**ので、入れ直しても
/// 古い DLL が使われ続ける。しかも症状はアプリによって出たり出なかったり
/// するので、原因として疑いにくい。
fn clear_per_user_registration() {
    let Some(stale) = crystalskk_tip::registry::per_user_dll_path() else {
        return;
    };
    match crystalskk_tip::registry::unregister_per_user_class() {
        Ok(()) => println!("古い利用者ごとの登録を消しました: {stale}"),
        Err(e) => eprintln!("crystalskk-setup: 古い登録を消せません: {}", e.message()),
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
            // 昇格した側の報告を鵜呑みにしない。**本人として見た結果**を
            // 確かめる。昇格した側の `HKEY_CURRENT_USER` は別人のものかも
            // しれず、向こうから見えている景色は当てにならない。
            confirm_effective_registration();
            if code == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => fail(&e.to_string(), None),
    }
}

fn do_install(source: Option<&Path>, ranker_from: Option<&Path>, report: &Report) -> ExitCode {
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
            install_server(&installed.dll, ranker_from, report);

            report.say("\n");
            report.say("設定 → 時刻と言語 → 言語と地域 → 日本語 → 言語のオプション →\n");
            report.say("キーボード に CrystalSKK が現れます。\n");
            // 昇格して呼ばれたときは、親のほうが本人として確かめ直す。
            if report.is_console() {
                confirm_effective_registration();
            }
            // かつて辞書に与えた許可を外す。**辞書サーバができて要らなく
            // なった** (ADR-0016) ので、与えたままにしておく理由がない。
            dictionary::revoke_access();
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.to_string(), Some(report)),
    }
}

/// 辞書サーバを入れ替え、起こし直す。
///
/// **辞書を持っているのはサーバだけ** (ADR-0016) なので、これが居ないと
/// 変換が一件も引けない。入れ替えたらその場で起こす。
fn install_server(dll: &Path, ranker_from: Option<&Path>, report: &Report) {
    let Some(directory) = dll.parent() else {
        return;
    };
    let Some(source) = server::default_source() else {
        report.say("\n");
        report.say("辞書サーバが見つかりません。変換ができません。\n");
        report.say("cargo build -p crystalskk-server --release を実行してください。\n");
        return;
    };

    match install::install_server(&source, directory) {
        Ok(stopped) => {
            if stopped {
                report.say("\n");
                report.say("動いていた辞書サーバに終わってもらいました。\n");
            }
        }
        Err(e) => {
            report.say(&format!("\n辞書サーバを入れ替えられません: {e}\n"));
            return;
        }
    }

    // ログオンのたびに起きるようにする。隔離されたアプリからは起こせない
    // ので、**居ない場面を作らない**ことが効く。
    if let Err(e) = server::register_autostart(directory) {
        report.say(&format!("自動起動を登録できません: {e}\n"));
    }

    // 言語モデル一式は、辞書サーバが止まっているうちに入れ替える。
    // 動いているサーバは DLL とモデルを握っている。
    ranker::install(
        &crystalskk_server::paths::ranker_dir(directory),
        ranker_from,
        report,
    );

    match server::start(directory) {
        Ok(()) => report.say("辞書サーバを起こしました。\n"),
        Err(e) => report.say(&format!("辞書サーバを起こせません: {e}\n")),
    }
}

/// 結局どの DLL が使われるのかを、本人の目線で確かめて出す。
///
/// 導入が「成功しました」と言っても、実際に読み込まれるのが別の DLL で
/// あることがある。利用者ごとの登録が残っていると、そちらが優先される。
/// **黙って成功を報告すると、それが何時間も隠れる。**
fn confirm_effective_registration() {
    let status = install::status();
    let Some(effective) = status.effective() else {
        println!();
        println!("登録が見当たりません。install が通っていません。");
        return;
    };

    println!();
    println!("実際に使われる DLL: {}", effective.display());

    if status.per_user.is_some() {
        println!();
        println!("利用者ごとの登録が残っています。**こちらが優先されます。**");
        println!("機械全体へ入れ替えても、この DLL は使われません。");
        println!("消せていないので、もう一度 install してください。");
    }
}

fn do_uninstall(purge: bool, report: &Report) -> ExitCode {
    // 先に辞書サーバに終わってもらう。握られたままではファイルを消せず、
    // 残しておく意味もない。
    if server::stop() {
        report.say("辞書サーバに終わってもらいました。\n");
    }
    server::unregister_autostart();

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

/// 記録の段階を、目印のファイルに書く。
///
/// 環境変数ではなくファイルにするのは、**包装されたアプリに環境変数が
/// 届かない**ため。目印は DLL の隣に置く。そこなら隔離された入れ物の中
/// からも読める。
fn do_log(level: Level, report: &Report) -> ExitCode {
    let directory = match install::install_dir() {
        Ok(directory) => directory,
        Err(e) => return fail(&e.to_string(), Some(report)),
    };
    let marker = directory.join(crystalskk_tip::log::MARKER_NAME);

    // 切るときも目印は残す。**消すと「指定なし」に戻り、既定の段階で
    // 記録が再開してしまう。**
    let result = std::fs::write(&marker, level.name().as_bytes());
    if let Err(e) = result {
        return fail(&e.to_string(), Some(report));
    }

    if level == Level::Off {
        report.say("記録をやめます。アプリを開き直すと効きます。\n");
        report.say("\n");
        report.say("環境変数 CRYSTALSKK_LOG を設定している場合は、そちらも消してください。\n");
        report.say("詳しいほうが採られます。\n");
        return ExitCode::SUCCESS;
    }

    report.say(&format!(
        "記録の段階を {} にしました。アプリを開き直すと効きます。\n",
        level.name()
    ));
    report.say("\n");
    report.say(concat!(r"記録先: %LOCALAPPDATA%\CrystalSKK\tip.log", "\n"));
    report.say("包装されたアプリ (MSIX) では、そこではなく\n");
    report.say(concat!(
        r"%LOCALAPPDATA%\Packages\<包装の名前>\AC\CrystalSKK\tip.log",
        " に落ちます。\n"
    ));
    if level == Level::Trace {
        report.say("\n");
        report.say("trace は打鍵のたびにファイルへ書きます。**入力が重くなります。**\n");
        report.say("確認が済んだら log info か log off に戻してください。\n");
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
    /// 診断の記録の細かさを決める。
    Log(Level),
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
    /// 言語モデル一式 (`build.py` の出力) のあるフォルダ。
    ranker_from: Option<PathBuf>,
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
                    let which = rest
                        .next()
                        .ok_or("log には段階が要ります (off / error / info / trace)")?;
                    // 綴りを外した値は、いちばん詳しいところへ倒れる。
                    // 記録を頼んだ人を黙って無視するよりはよい。
                    set(&mut command, Command::Log(Level::parse(which)))?;
                }
                "--purge" => parsed.purge = true,
                "--no-elevate" => parsed.no_elevate = true,
                "--ranker-from" => {
                    let path = rest
                        .next()
                        .ok_or("--ranker-from に言語モデル一式のフォルダが要ります")?;
                    // 報告にはどこから写したかを出す。相対のままだと分かりにくい。
                    let path = std::path::absolute(path).map_err(|e| format!("{path}: {e}"))?;
                    parsed.ranker_from = Some(path);
                }
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
        if parsed.ranker_from.is_some() && command != Command::Install {
            return Err("--ranker-from は install にだけ使えます".to_owned());
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
      --ranker-from <フォルダ>        候補を並べる言語モデル一式の場所
                                      (省略時は target/ranker-model/out を探す)
  crystalskk-setup uninstall          登録を解除する
  crystalskk-setup uninstall --purge  写した DLL も削除する
  crystalskk-setup status             今の状態を表示する
  crystalskk-setup dict               設定に並べた辞書をいま取り直す (権限は要らない)
  crystalskk-setup log <段階>         診断の記録の細かさを決める
                                      off / error / info / trace

log の段階: off (既定) / error (失敗だけ) / info (節目の出来事) /
trace (打鍵ごと。入力が重くなる)

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
    fn reads_the_log_level() {
        for (word, expected) in [
            ("off", Level::Off),
            ("error", Level::Error),
            ("info", Level::Info),
            ("trace", Level::Trace),
        ] {
            let parsed = parse(&["log", word]).expect("読める").expect("命令がある");
            assert_eq!(parsed.command, Command::Log(expected), "{word}");
        }
    }

    #[test]
    fn log_needs_a_level() {
        assert!(parse(&["log"]).is_err());
    }

    #[test]
    fn a_misspelled_level_records_everything() {
        // 黙って無視されるより、出しすぎるほうがまだよい。
        let parsed = parse(&["log", "verbose"])
            .expect("読める")
            .expect("命令がある");
        assert_eq!(parsed.command, Command::Log(Level::Trace));
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
