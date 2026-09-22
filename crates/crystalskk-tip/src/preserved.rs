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
/// 入と同じキーを並べるのは、**同じキーが入切を兼ねる**ため。TSF は
/// 先に登録したほうを優先し、後から同じ組み合わせを登録しても上書き
/// されない。押されたとき今の状態を見て決めるので、両方に出しておく。
const TURN_OFF: &[(u16, u32)] = &[
    (VK_OEM_3.0, TF_MOD_ALT),
    (VK_KANJI.0, TF_MOD_IGNORE_ALL_MODIFIER),
    (VK_OEM_AUTO.0, TF_MOD_IGNORE_ALL_MODIFIER),
    (VK_OEM_ENLW.0, TF_MOD_IGNORE_ALL_MODIFIER),
    (VK_IME_OFF.0, TF_MOD_IGNORE_ALL_MODIFIER),
];

/// 入切のキーを登録する。
///
/// 一つも登録できなくても有効化そのものは続ける。入切ができないだけで、
/// すでに入っているアプリでは入力できる。
pub fn register(keystrokes: &ITfKeystrokeMgr, client_id: u32) {
    // 切を先に登録する。同じキーが重なったとき、押して最初に効くのが
    // 「切」になるようにする。入っている状態から押すのが普通の順番で、
    // 切られた状態からは入の側が拾う。
    let off = register_set(keystrokes, client_id, &GUID_PRESERVED_KEY_OFF, TURN_OFF, "OFF");
    let on = register_set(keystrokes, client_id, &GUID_PRESERVED_KEY_ON, TURN_ON, "ON");
    log::write(&format!("入切のキーを登録した (入 {on} 件, 切 {off} 件)"));
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
fn unregister_set(
    keystrokes: &ITfKeystrokeMgr,
    client_id: u32,
    guid: &GUID,
    keys: &[(u16, u32)],
) {
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
