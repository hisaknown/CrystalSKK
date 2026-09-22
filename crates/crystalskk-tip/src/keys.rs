//! 仮想キーをエンジンのキーに直す。
//!
//! SKK は大文字と小文字を区別するため、仮想キーだけでは足りない。
//! シフトやキーボード配列を通した「実際に入る文字」が要る。それを得るには
//! `ToUnicodeEx` に聞くのが唯一まともな方法で、自前で A〜Z を並べると
//! 英語配列以外で壊れる。

use crystalskk_core::Key;
use windows::Win32::Foundation::WPARAM;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, GetKeyboardState, ToUnicodeEx, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DOWN,
    VK_ESCAPE, VK_RETURN, VK_SPACE, VK_TAB, VK_UP,
};

/// キーボードの状態を読む長さ。`GetKeyboardState` が定める。
const KEY_STATE_LEN: usize = 256;

/// 押されている印。仮想キーの状態の最上位ビット。
const KEY_PRESSED: u8 = 0x80;

/// `ToUnicodeEx` にキーボードの状態を書き換えさせない指定。
///
/// 「押されるか試す」問い合わせで状態を変えてしまうと、次の打鍵が狂う。
const DONT_CHANGE_STATE: u32 = 0x4;

/// 仮想キーをエンジンのキーに直す。対応するものがなければ `None`。
pub fn translate(wparam: WPARAM) -> Option<Key> {
    let virtual_key = VIRTUAL_KEY(u16::try_from(wparam.0 & 0xFFFF).ok()?);

    // 位置の決まっているキーは、文字に直さずそのまま対応づける。
    match virtual_key {
        VK_RETURN => return Some(Key::Enter),
        VK_BACK => return Some(Key::Backspace),
        VK_ESCAPE => return Some(Key::Escape),
        VK_TAB => return Some(Key::Tab),
        VK_UP => return Some(Key::Up),
        VK_DOWN => return Some(Key::Down),
        VK_SPACE => return Some(Key::Space),
        _ => {}
    }

    let mut state = [0u8; KEY_STATE_LEN];
    // SAFETY: 定められた長さの配列をそのまま渡している。
    if unsafe { GetKeyboardState(&mut state) }.is_err() {
        return None;
    }

    // Ctrl 付きの英字は、文字に直すと制御文字になってしまうので先に拾う。
    if state[VK_CONTROL.0 as usize] & KEY_PRESSED != 0 {
        let letter = ascii_letter(virtual_key)?;
        return Some(Key::Ctrl(letter));
    }

    to_character(virtual_key, &state).map(Key::Char)
}

/// 仮想キーが英字なら、その小文字。
///
/// Ctrl 付きの判定にだけ使う。ここは配列に依らず、英字の仮想キーが
/// `'A'`〜`'Z'` の値を持つという決まりに頼ってよい。
fn ascii_letter(virtual_key: VIRTUAL_KEY) -> Option<char> {
    let code = virtual_key.0;
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&code) {
        char::from_u32(u32::from(code)).map(|c| c.to_ascii_lowercase())
    } else {
        None
    }
}

/// キーボード配列とシフトの状態を通して、実際に入る文字を得る。
fn to_character(virtual_key: VIRTUAL_KEY, state: &[u8; KEY_STATE_LEN]) -> Option<char> {
    let mut buffer = [0u16; 8];
    // SAFETY: 状態と書き込み先は長さ付きで渡しており、状態は書き換えさせない。
    let written = unsafe {
        let layout = GetKeyboardLayout(0);
        ToUnicodeEx(
            u32::from(virtual_key.0),
            0,
            state,
            &mut buffer,
            DONT_CHANGE_STATE,
            Some(layout),
        )
    };

    // 負の値は死にキー。一文字にならないものは扱わない。
    if written != 1 {
        return None;
    }
    let character = char::from_u32(u32::from(buffer[0]))?;
    // 制御文字はエンジンの知らないものなので落とす。
    if character.is_control() {
        None
    } else {
        Some(character)
    }
}
