//! 入切のキーを横取りする。
//!
//! 入力方式が切にされているとき、打鍵は CrystalSKK に届かない。では
//! どうやって入に戻すのか。**入切のキーだけは別扱いで受け取る**のが
//! TSF の作法で、これを「横取りキー」(preserved key) という。
//!
//! 横取りキーは打鍵の受け口 ([`windows::Win32::UI::TextServices::ITfKeyEventSink`])
//! を通らない。切られていても押されたことが分かる。
//!
//! # 入にする鍵を持たないとどうなるか
//!
//! **入力方式は切られたまま、二度と戻らない。** アプリが起動時に切って
//! いれば、そのアプリでは日本語が一文字も打てない。利用者から見れば
//! 「このアプリでは動かない」としか映らない。
//!
//! 登録するキーは設定の `[keys]` の `on_off`・`on`・`off` で決まる
//! (ADR-0038)。雛形は CorvusSKK に倣い、半角/全角、Alt+`、そして IME 専用の
//! キーを並べている。機種や設定で押せるキーが違うため、**一つに絞らず全部
//! 拾う**。
//!
//! 設定を受け取るまでは何も登録しない。受け取るまではエンジンも動かない
//! (ADR-0020) ので、入にできても打てるものは無い。

use crystalskk_settings::{Hotkey, HotkeyKey, OnOffKeys};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, VK_F1, VK_IME_OFF, VK_IME_ON, VK_KANJI, VK_OEM_AUTO, VK_OEM_ENLW, VK_SPACE,
    VkKeyScanExW,
};
use windows::Win32::UI::TextServices::{
    ITfKeystrokeMgr, TF_MOD_ALT, TF_MOD_CONTROL, TF_MOD_IGNORE_ALL_MODIFIER, TF_MOD_SHIFT,
    TF_PRESERVEDKEY,
};
use windows::core::GUID;

use crate::guids::{GUID_PRESERVED_KEY_OFF, GUID_PRESERVED_KEY_ON};
use crate::log;

/// 登録した横取りキー。外すときに同じものを渡す。
pub type Registered = Vec<(GUID, TF_PRESERVEDKEY)>;

/// 入切のキーを登録し、登録できたものを返す。
///
/// `open` にはいまの状態を渡す。**入切が変わるたびに登録し直す必要がある。**
///
/// TSF は同じ組み合わせを二度登録させない。先に登録したほうが勝ち、後から
/// 同じキーを別の用途で登録しても黙って無視される。半角/全角のように入切を
/// 兼ねるキーでは、**どちらを先に登録するかがそのままキーの意味になる**。
///
/// だから、いまの状態と逆のほうを先に登録する。入っているなら「切」が、
/// 切れているなら「入」が、そのキーを取る。押せば必ず反対側へ移る。
///
/// 一つも登録できなくても有効化そのものは続ける。入切ができないだけで、
/// すでに入っているアプリでは入力できる。
pub fn register(
    keystrokes: &ITfKeystrokeMgr,
    client_id: u32,
    open: bool,
    keys: &OnOffKeys,
) -> Registered {
    // 入切を兼ねるキーは、どちらの側にも並べる。
    let turn_on: Vec<TF_PRESERVEDKEY> = keys
        .toggle
        .iter()
        .chain(&keys.on)
        .flat_map(preserved_keys)
        .collect();
    let turn_off: Vec<TF_PRESERVEDKEY> = keys
        .toggle
        .iter()
        .chain(&keys.off)
        .flat_map(preserved_keys)
        .collect();

    let on = (&GUID_PRESERVED_KEY_ON, turn_on, "ON");
    let off = (&GUID_PRESERVED_KEY_OFF, turn_off, "OFF");
    let (first, second) = if open { (off, on) } else { (on, off) };

    let mut registered = Registered::new();
    let taken = register_set(
        keystrokes,
        client_id,
        first.0,
        &first.1,
        first.2,
        &mut registered,
    );
    let rest = register_set(
        keystrokes,
        client_id,
        second.0,
        &second.1,
        second.2,
        &mut registered,
    );
    log::write(&format!(
        "入切のキーを登録した (いまは{}。{} が {taken} 件、{} が {rest} 件)",
        if open { "入" } else { "切" },
        first.2,
        second.2
    ));
    registered
}

/// 入切のキーの登録を外す。外せなくても続ける。
pub fn unregister(keystrokes: &ITfKeystrokeMgr, registered: &Registered) {
    for (guid, key) in registered {
        // SAFETY: 登録したときと同じ組み合わせを渡している。
        unsafe {
            let _ = keystrokes.UnpreserveKey(guid, key);
        }
    }
}

/// 一組を登録し、通った数を返す。
fn register_set(
    keystrokes: &ITfKeystrokeMgr,
    client_id: u32,
    guid: &GUID,
    keys: &[TF_PRESERVEDKEY],
    description: &str,
    registered: &mut Registered,
) -> usize {
    let description: Vec<u16> = description.encode_utf16().collect();
    let mut count = 0;
    for key in keys {
        // SAFETY: GUID も説明も呼び出しの間だけ使われる。
        if unsafe { keystrokes.PreserveKey(client_id, guid, key, &description) }.is_ok() {
            registered.push((*guid, *key));
            count += 1;
        }
    }
    count
}

/// 設定のキー一つを、横取りするキーの組み合わせに直す。
///
/// 半角/全角は配列や修飾によって届く仮想キーが違うので、幾つにもなる。
/// 文字のキーはいまの配列で仮想キーを引く。**その文字を打つのにシフトが
/// 要るなら、シフトも修飾に足す。** 配列に無い文字なら何も返さない。
fn preserved_keys(hotkey: &Hotkey) -> Vec<TF_PRESERVEDKEY> {
    let any = TF_MOD_IGNORE_ALL_MODIFIER;
    let mut modifiers = 0;
    if hotkey.ctrl {
        modifiers |= TF_MOD_CONTROL;
    }
    if hotkey.shift {
        modifiers |= TF_MOD_SHIFT;
    }
    if hotkey.alt {
        modifiers |= TF_MOD_ALT;
    }
    let keys: Vec<(u16, u32)> = match hotkey.key {
        HotkeyKey::HankakuZenkaku => vec![
            (VK_KANJI.0, any),
            (VK_OEM_AUTO.0, any),
            (VK_OEM_ENLW.0, any),
        ],
        HotkeyKey::ImeOn => vec![(VK_IME_ON.0, any)],
        HotkeyKey::ImeOff => vec![(VK_IME_OFF.0, any)],
        HotkeyKey::Space => vec![(VK_SPACE.0, modifiers)],
        HotkeyKey::Function(n) => vec![(VK_F1.0 + u16::from(n) - 1, modifiers)],
        HotkeyKey::Char(c) => {
            let mut buffer = [0u16; 2];
            let units = c.encode_utf16(&mut buffer);
            if units.len() != 1 {
                return Vec::new();
            }
            // SAFETY: いまのスレッドの配列を問い合わせるだけ。
            let scanned = unsafe { VkKeyScanExW(units[0], GetKeyboardLayout(0)) };
            if scanned == -1 {
                log::write(&format!("入切のキー「{c}」はいまの配列にありません"));
                return Vec::new();
            }
            let scanned = scanned as u16;
            let mut modifiers = modifiers;
            if scanned & 0x100 != 0 {
                modifiers |= TF_MOD_SHIFT;
            }
            vec![(scanned & 0xFF, modifiers)]
        }
    };
    keys.into_iter()
        .map(|(vkey, modifiers)| TF_PRESERVEDKEY {
            uVKey: u32::from(vkey),
            uModifiers: modifiers,
        })
        .collect()
}
