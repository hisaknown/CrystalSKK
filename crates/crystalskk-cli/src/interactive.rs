//! 対話モード。
//!
//! 端末を raw mode にして一打鍵ずつエンジンへ渡す。実際の IME に近い
//! 手触りを確かめるためのもので、行入力モードと同じセッションを使う。
//!
//! Ctrl+C で終了する。SKK が使う Ctrl+J / Ctrl+G / Ctrl+Q は、そのまま
//! エンジンへ渡る。

use std::io::{self, Write};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};
use crystalskk_core::Key;

use crate::render;
use crate::session::Session;

/// 状態表示に使う行数。描き直しのたびにこの数だけ遡る。
const PANEL_LINES: u16 = 4;

pub fn run(session: &mut Session) -> io::Result<()> {
    let mut out = io::stdout();
    writeln!(out, "対話モードです。Ctrl+C で終了します。")?;
    writeln!(out)?;

    terminal::enable_raw_mode()?;
    let result = drive(session, &mut out);
    // 失敗しても raw mode は必ず戻す。端末を壊したまま終わらせない。
    let restored = terminal::disable_raw_mode();

    draw_final(session, &mut out)?;
    result.and(restored.map_err(io::Error::other))
}

fn drive(session: &mut Session, out: &mut impl Write) -> io::Result<()> {
    let mut first = true;
    loop {
        draw(session, out, first)?;
        first = false;

        let Event::Key(event) = crossterm::event::read()? else {
            continue;
        };
        // 押した瞬間だけを見る。Windows では離したときにも届く。
        if event.kind != KeyEventKind::Press {
            continue;
        }
        if is_quit(&event) {
            return Ok(());
        }
        if let Some(key) = translate(&event) {
            session.press(key);
        }
    }
}

/// 終了の合図か。SKK が使わない Ctrl+C を充てる。
fn is_quit(event: &KeyEvent) -> bool {
    event.modifiers.contains(KeyModifiers::CONTROL) && matches!(event.code, KeyCode::Char('c'))
}

/// 端末のキーをエンジンのキーに直す。
fn translate(event: &KeyEvent) -> Option<Key> {
    let key = match event.code {
        KeyCode::Char(' ') => Key::Space,
        KeyCode::Char(c) if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Key::Ctrl(c.to_ascii_lowercase())
        }
        // シフトの有無は、届く文字が大文字かどうかにそのまま表れる。
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Tab => Key::Tab,
        KeyCode::Esc => Key::Escape,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        _ => return None,
    };
    Some(key)
}

fn draw(session: &Session, out: &mut impl Write, first: bool) -> io::Result<()> {
    if !first {
        execute!(out, cursor::MoveUp(PANEL_LINES))?;
    }
    execute!(
        out,
        cursor::MoveToColumn(0),
        terminal::Clear(terminal::ClearType::FromCursorDown)
    )?;

    let candidates = match session.candidates() {
        Some(view) => {
            let mut line = render::candidates(&view);
            if let Some(annotation) = render::annotation(&view) {
                line.push_str(&format!("   ; {annotation}"));
            }
            line
        }
        None => String::new(),
    };

    // raw mode では改行に \r\n が要る。
    write!(out, "  文書:   {}\r\n", render::document(session))?;
    write!(out, "  未確定: {}\r\n", render::preedit(session))?;
    write!(out, "  候補:   {candidates}\r\n")?;
    write!(out, "  モード: {}\r\n", render::mode(session))?;
    out.flush()
}

/// raw mode を抜けたあとに、最後の状態をもう一度出す。
fn draw_final(session: &Session, out: &mut impl Write) -> io::Result<()> {
    writeln!(out)?;
    writeln!(out, "文書: {}", render::document(session))?;
    out.flush()
}
