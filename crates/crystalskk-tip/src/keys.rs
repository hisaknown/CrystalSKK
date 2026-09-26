//! 仮想キーをエンジンのキーに直す。
//!
//! SKK は大文字と小文字を区別するため、仮想キーだけでは足りない。
//! シフトやキーボード配列を通した「実際に入る文字」が要る。それを得るには
//! `ToUnicodeEx` に聞くのが唯一まともな方法で、自前で A〜Z を並べると
//! 英語配列以外で壊れる。
//!
//! # 修飾キーは二度確かめる
//!
//! `GetKeyboardState` が返す配列は、修飾キーの状態を落としていることが
//! ある。実際に Ctrl+J が素の `j` として届き、英数モードから戻れなく
//! なった。TIP はアプリのメッセージ処理の途中で呼ばれるため、そのときの
//! 待ち行列の状態が同期されているとは限らない。
//!
//! そこで、配列を受け取ったあとで修飾キーだけ `GetKeyState` で上書きする。
//! こうすると修飾の判定だけでなく、`ToUnicodeEx` が返す文字も正しくなる。

use crystalskk_core::Key;
use windows::Win32::Foundation::WPARAM;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardLayout, GetKeyboardState, INPUT, INPUT_0, INPUT_KEYBOARD,
    KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, ToUnicodeEx, VIRTUAL_KEY, VK_BACK,
    VK_CAPITAL, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_INSERT, VK_LCONTROL, VK_LMENU, VK_LSHIFT,
    VK_MENU, VK_RCONTROL, VK_RETURN, VK_RMENU, VK_RSHIFT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};

use windows::Win32::UI::WindowsAndMessaging::GetMessageExtraInfo;

use crate::log;

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

    let state = keyboard_state()?;

    let control = state[VK_CONTROL.0 as usize] & KEY_PRESSED != 0;
    let shift = state[VK_SHIFT.0 as usize] & KEY_PRESSED != 0;

    // Windows の貼り付けは二通りある。どちらも同じ「貼り付け」に揃える。
    if virtual_key == VK_INSERT {
        return (shift && !control).then_some(Key::Paste);
    }

    // Ctrl 付きの英字は、文字に直すと制御文字になってしまうので先に拾う。
    if control {
        let letter = ascii_letter(virtual_key)?;
        if letter == 'v' {
            return Some(Key::Paste);
        }
        return Some(Key::Ctrl(letter));
    }

    to_character(virtual_key, &state).map(Key::Char)
}

/// いまのキーボードの状態。修飾キーは個別に確かめ直す。
fn keyboard_state() -> Option<[u8; KEY_STATE_LEN]> {
    let mut state = [0u8; KEY_STATE_LEN];
    // SAFETY: 定められた長さの配列をそのまま渡している。
    if unsafe { GetKeyboardState(&mut state) }.is_err() {
        return None;
    }

    // 配列が修飾キーを落としていることがあるので、ここだけ上書きする。
    for key in MODIFIERS {
        // SAFETY: 仮想キーの番号を渡して状態を問い合わせるだけ。
        let pressed = unsafe { GetKeyState(i32::from(key.0)) } < 0;
        if pressed {
            state[key.0 as usize] |= KEY_PRESSED;
        }
    }
    // 左右どちらかが押されていれば、まとめの側も押されているとみなす。
    merge_side(&mut state, VK_CONTROL, VK_LCONTROL, VK_RCONTROL);
    merge_side(&mut state, VK_SHIFT, VK_LSHIFT, VK_RSHIFT);
    merge_side(&mut state, VK_MENU, VK_LMENU, VK_RMENU);

    Some(state)
}

/// 左右の別を、まとめの仮想キーへ反映する。
fn merge_side(
    state: &mut [u8; KEY_STATE_LEN],
    both: VIRTUAL_KEY,
    left: VIRTUAL_KEY,
    right: VIRTUAL_KEY,
) {
    let pressed = (state[left.0 as usize] | state[right.0 as usize]) & KEY_PRESSED;
    state[both.0 as usize] |= pressed;
}

/// 確かめ直す修飾キー。
const MODIFIERS: [VIRTUAL_KEY; 10] = [
    VK_CONTROL,
    VK_LCONTROL,
    VK_RCONTROL,
    VK_SHIFT,
    VK_LSHIFT,
    VK_RSHIFT,
    VK_MENU,
    VK_LMENU,
    VK_RMENU,
    VK_CAPITAL,
];

/// 打鍵の解釈を記録する。何がどう見えているかを外から確かめるため。
pub fn log_translation(wparam: WPARAM, key: Option<Key>) {
    // 打鍵ごとに呼ばれる。記録しないと決まっているなら、修飾キーを
    // 調べるところから省く。
    if !log::tracing() {
        return;
    }
    let virtual_key = wparam.0 & 0xFFFF;
    let state = keyboard_state();
    let pressed = |k: VIRTUAL_KEY| state.is_some_and(|s| s[k.0 as usize] & KEY_PRESSED != 0);
    log::trace(&format!(
        "キー VK={virtual_key:#04x} Ctrl={} Shift={} Alt={} → {key:?}",
        pressed(VK_CONTROL),
        pressed(VK_SHIFT),
        pressed(VK_MENU),
    ));
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

/// 送り直した打鍵に付ける印。`dwExtraInfo` に入れて、戻ってきたときに見分ける。
///
/// 値に意味は無い。ほかのソフトの印と重ならなければよい ("CSKK")。
const RESENT_MARK: usize = 0x4353_4B4B;

/// 確定したあとで、同じキーをアプリへ送り直す (ADR-0039)。
///
/// `OnKeyDown` で食べなかったと答えるだけでは、アプリによっては届かない。
/// Firefox は `OnTestKeyDown` の答えを見てキーを捨ててしまう。そこで一度
/// 食べ、確定を書いてから本物のキーとして送り直す。修飾キーは押されたまま
/// なので、送るのは仮想キーの押し下げと離しだけでよい。
pub fn resend(wparam: WPARAM) {
    let Ok(code) = u16::try_from(wparam.0 & 0xFFFF) else {
        return;
    };
    let input = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(code),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: RESENT_MARK,
            },
        },
    };
    let inputs = [input(KEYBD_EVENT_FLAGS(0)), input(KEYEVENTF_KEYUP)];
    // SAFETY: 長さ付きの配列と、その一つの大きさを渡している。
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        log::error(&format!("キーを送り直せなかった (VK={code:#04x})"));
    }
}

/// いま処理している打鍵は、こちらが送り直したものか。
///
/// そうなら何も見ずにアプリへ渡す。**送り直したキーをまた食べると、
/// 確定のたびに同じキーが回り続ける。**
pub fn is_resent() -> bool {
    // SAFETY: このスレッドが最後に受け取ったメッセージの付加情報を読むだけ。
    unsafe { GetMessageExtraInfo() }.0 as usize == RESENT_MARK
}
