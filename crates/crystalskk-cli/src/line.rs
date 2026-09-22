//! 行入力モード。
//!
//! 一行を打鍵列として受け取り、処理後の状態を表示する。端末を占有しない
//! ので、パイプで流し込んで結果を確かめることもできる。

use std::io::{self, BufRead, Write};

use crate::session::Session;
use crate::{keys, render};

/// 標準入力を最後まで読んで処理する。
pub fn run(session: &mut Session, interactive_prompt: bool) -> io::Result<()> {
    let stdin = io::stdin();
    let mut out = io::stdout().lock();

    if interactive_prompt {
        writeln!(
            out,
            "打鍵列を入力してください。:help で説明、:quit で終了。"
        )?;
    }

    for line in stdin.lock().lines() {
        let line = line?;
        // 端末から使っているときは、打った内容がすでに見えている。
        if !interactive_prompt {
            writeln!(out, "> {line}")?;
        }

        if let Some(command) = line.strip_prefix(':') {
            match execute(session, command, &mut out)? {
                Flow::Continue => {}
                Flow::Quit => break,
            }
            prompt(&mut out, interactive_prompt)?;
            continue;
        }

        match keys::parse(&line) {
            Ok(parsed) => {
                session.press_all(parsed);
                report(session, &mut out)?;
            }
            Err(message) => writeln!(out, "  入力を読めません: {message}")?,
        }
        prompt(&mut out, interactive_prompt)?;
    }
    Ok(())
}

fn prompt(out: &mut impl Write, show: bool) -> io::Result<()> {
    if show {
        write!(out, "> ")?;
        out.flush()?;
    }
    Ok(())
}

/// 一行処理したあとの状態。
fn report(session: &Session, out: &mut impl Write) -> io::Result<()> {
    writeln!(out, "  文書:   {}", render::document(session))?;
    writeln!(out, "  未確定: {}", render::preedit(session))?;
    if let Some(view) = session.candidates() {
        writeln!(out, "  候補:   {}", render::candidates(&view))?;
        if let Some(annotation) = render::annotation(&view) {
            writeln!(out, "  注釈:   {annotation}")?;
        }
    }
    writeln!(out, "  モード: {}", render::mode(session))?;
    if let Some(key) = session.last_unhandled() {
        writeln!(out, "  素通し: {}", keys::display(key))?;
    }
    Ok(())
}

enum Flow {
    Continue,
    Quit,
}

fn execute(session: &mut Session, command: &str, out: &mut impl Write) -> io::Result<Flow> {
    let mut parts = command.split_whitespace();
    match parts.next().unwrap_or("") {
        "q" | "quit" => return Ok(Flow::Quit),
        "h" | "help" => write!(out, "{HELP}")?,
        "clear" => {
            session.clear_document();
            writeln!(out, "  文書を空にしました")?;
        }
        "state" => report(session, out)?,
        "save" => match session.save_user_dictionary() {
            Ok(true) => writeln!(
                out,
                "  ユーザー辞書を保存しました: {}",
                session.user_dictionary_path().display()
            )?,
            Ok(false) => writeln!(out, "  変更がないので保存しませんでした")?,
            Err(e) => writeln!(out, "  保存に失敗しました: {e}")?,
        },
        other => writeln!(out, "  知らない命令です: :{other}  (:help を参照)")?,
    }
    Ok(Flow::Continue)
}

const HELP: &str = "\
  打鍵列の表記
    印字可能文字   そのまま。英大文字はシフト付きの打鍵 (見出し語・送り仮名の開始)
    空白           Space (変換・次候補)
    \\n             Enter        \\b  Backspace   \\t  Tab
    \\e             Escape       \\u  ↑           \\d  ↓
    ^J ^G ^Q       Ctrl 付きの打鍵
    \\\\ \\^          \\ と ^ そのもの

  例
    Kanji          → ▽かんじ
    Kanji\\s        → ▼漢字        (\\s は空白と同じ)
    OkuRi          → ▼送り
    /skk\\s         → ▼SKK

  命令
    :help   この説明          :state  今の状態
    :clear  文書を空にする    :save   ユーザー辞書を保存
    :quit   終了
";
