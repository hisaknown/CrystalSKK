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
//! 登録するキーは CorvusSKK に倣う。半角/全角、Alt+`、そして IME 専用の
//! キーを並べる。機種や設定で押せるキーが違うため、**一つに絞らず全部
//! 拾う**。

use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_IME_OFF, VK_IME_ON, VK_KANJI, VK_OEM_3, VK_OEM_AUTO, VK_OEM_ENLW,
};
use windows::Win32::UI::TextServices::{
    ITfKeystrokeMgr, TF_MOD_ALT, TF_MOD_IGNORE_ALL_MODIFIER, TF_PRESERVEDKEY,
};
use windows::core::GUID;

use crate::guids::{GUID_PRESERVED_KEY_OFF, GUID_PRESERVED_KEY_ON};
use crate::log;

/// 入にするキー。
const TURN_ON: &[(u16, u32)] = &[
    // Alt + 半角/全角の位置にあるキー。
    (VK_OEM_3.0, TF_MOD_ALT),
    // 半角/全角。修飾は問わない。
    (VK_KANJI.0, TF_MOD_IGNORE_ALL_MODIFIER),
    (VK_OEM_AUTO.0, TF_MOD_IGNORE_ALL_MODIFIER),
    (VK_OEM_ENLW.0, TF_MOD_IGNORE_ALL_MODIFIER),
    // IME を入にする専用のキー。持っている配列だけが送ってくる。
    (VK_IME_ON.0, TF_MOD_IGNORE_ALL_MODIFIER),
];

/// 切にするキー。
///
/// 入とほとんど同じ顔ぶれになる。**半角/全角は一つのキーで入切を兼ねる**
/// ので、どちらの側にも並べるしかない。違うのは最後の一つだけで、そちらは
/// 入専用・切専用のキーである。
const TURN_OFF: &[(u16, u32)] = &[
    (VK_OEM_3.0, TF_MOD_ALT),
    (VK_KANJI.0, TF_MOD_IGNORE_ALL_MODIFIER),
    (VK_OEM_AUTO.0, TF_MOD_IGNORE_ALL_MODIFIER),
    (VK_OEM_ENLW.0, TF_MOD_IGNORE_ALL_MODIFIER),
    (VK_IME_OFF.0, TF_MOD_IGNORE_ALL_MODIFIER),
];

/// 入切のキーを登録する。
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
pub fn register(keystrokes: &ITfKeystrokeMgr, client_id: u32, open: bool) {
    let (first, second) = if open {
        (
            (&GUID_PRESERVED_KEY_OFF, TURN_OFF, "OFF"),
            (&GUID_PRESERVED_KEY_ON, TURN_ON, "ON"),
        )
    } else {
        (
            (&GUID_PRESERVED_KEY_ON, TURN_ON, "ON"),
            (&GUID_PRESERVED_KEY_OFF, TURN_OFF, "OFF"),
        )
    };

    let taken = register_set(keystrokes, client_id, first.0, first.1, first.2);
    let rest = register_set(keystrokes, client_id, second.0, second.1, second.2);
    log::write(&format!(
        "入切のキーを登録した (いまは{}。{} が {taken} 件、{} が {rest} 件)",
        if open { "入" } else { "切" },
        first.2,
        second.2
    ));
}

/// 入切のキーの登録を外す。
pub fn unregister(keystrokes: &ITfKeystrokeMgr, client_id: u32) {
    unregister_set(keystrokes, client_id, &GUID_PRESERVED_KEY_ON, TURN_ON);
    unregister_set(keystrokes, client_id, &GUID_PRESERVED_KEY_OFF, TURN_OFF);
}

/// 一組を登録し、通った数を返す。
fn register_set(
    keystrokes: &ITfKeystrokeMgr,
    client_id: u32,
    guid: &GUID,
    keys: &[(u16, u32)],
    description: &str,
) -> usize {
    let description: Vec<u16> = description.encode_utf16().collect();
    keys.iter()
        .filter(|(vkey, modifiers)| {
            let key = preserved_key(*vkey, *modifiers);
            // SAFETY: GUID も説明も呼び出しの間だけ使われる。
            unsafe { keystrokes.PreserveKey(client_id, guid, &key, &description) }.is_ok()
        })
        .count()
}

/// 一組の登録を外す。外せなくても続ける。
fn unregister_set(keystrokes: &ITfKeystrokeMgr, client_id: u32, guid: &GUID, keys: &[(u16, u32)]) {
    for (vkey, modifiers) in keys {
        let key = preserved_key(*vkey, *modifiers);
        // SAFETY: 登録したときと同じ組み合わせを渡している。
        unsafe {
            let _ = keystrokes.UnpreserveKey(guid, &key);
        }
        let _ = client_id;
    }
}

/// 横取りするキーの組み合わせを作る。
fn preserved_key(vkey: u16, modifiers: u32) -> TF_PRESERVEDKEY {
    TF_PRESERVEDKEY {
        uVKey: u32::from(vkey),
        uModifiers: modifiers,
    }
}
