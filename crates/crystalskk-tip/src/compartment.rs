//! 入力方式の入切と入力モードを、Windows とやり取りする。
//!
//! 区画 (compartment) は入力方式と外の世界が値を置き合う掲示板である。
//! こちらの状態を知らせるためだけのものではない。**外が書いた値を読む**
//! ためのものでもある。
//!
//! # 入切は向こうが決める
//!
//! `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` は入力方式が入か切かを表す。
//! これを書くのは CrystalSKK だけではない。利用者が入切のキーを押せば
//! Windows が、アプリが自分の都合で入力方式を切りたければアプリが書く。
//!
//! **どちらが本当かといえば、この区画のほうである。** こちらが「ひらがな
//! のつもり」でも、区画が切なら打鍵は届かない。だから読む。書いた値が
//! そのまま残っている前提で動いてはいけない (ADR-0012)。
//!
//! # 入力モードは知らせるだけ
//!
//! `..._INPUTMODE_CONVERSION` に書いた値が「ひらがな」「全角カタカナ」……
//! として読まれる。トレイの表示はこれを見ている。値の組み合わせは日本語
//! 入力の慣例に従い、[`conversion_mode`] が訳す。
//!
//! # 打鍵を食べてよい場面か
//!
//! 入力方式が入でも、いまの入力先が文字を受け取らないことがある。
//! [`accepts_input`] は、いま焦点のある文脈がそれを断っていないかを見る。

use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};
use windows::Win32::UI::TextServices::{
    GUID_COMPARTMENT_EMPTYCONTEXT, GUID_COMPARTMENT_KEYBOARD_DISABLED,
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION, GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE,
    GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, ITfCompartmentEventSink, ITfCompartmentMgr, ITfSource,
    ITfThreadMgr, TF_CONVERSIONMODE_ALPHANUMERIC, TF_CONVERSIONMODE_FULLSHAPE,
    TF_CONVERSIONMODE_KATAKANA, TF_CONVERSIONMODE_NATIVE, TF_CONVERSIONMODE_ROMAN,
    TF_SENTENCEMODE_PHRASEPREDICT,
};
use windows::core::{GUID, IUnknown, Interface};

use crystalskk_core::InputMode;

use crate::log;

/// 入力方式が入になっているか。
///
/// 読めないときは切とみなす。**入っていると決めてかかって打鍵を食べると、
/// 文字がどこにも出ないまま消える。** 分からないなら素通しするほうが害が
/// 小さい。
pub fn is_open(thread_manager: &ITfThreadMgr) -> bool {
    read(thread_manager, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE).is_some_and(|value| value != 0)
}

/// 入力方式の入切を書く。横取りキーで切り替えるときに使う。
pub fn set_open(thread_manager: &ITfThreadMgr, client_id: u32, open: bool) {
    let Ok(compartments) = thread_manager.cast::<ITfCompartmentMgr>() else {
        return;
    };
    write(
        &compartments,
        client_id,
        &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
        u32::from(open),
    );
}

/// いまの入力先が打鍵を受け取るか。
///
/// 焦点のある文書の一番上の文脈に、二つの断りの印が立っていないかを見る。
/// 文書も文脈も無いときは受け取らない。入力先が無いのだから、食べた文字は
/// 行き場を失う。
pub fn accepts_input(thread_manager: &ITfThreadMgr) -> bool {
    // SAFETY: 焦点を尋ねて、返ったものをその場で使うだけ。
    let context = unsafe {
        let Ok(documents) = thread_manager.GetFocus() else {
            return false;
        };
        match documents.GetTop() {
            Ok(context) => context,
            Err(_) => return false,
        }
    };

    let Ok(compartments) = context.cast::<ITfCompartmentMgr>() else {
        return false;
    };
    for guid in [
        GUID_COMPARTMENT_KEYBOARD_DISABLED,
        GUID_COMPARTMENT_EMPTYCONTEXT,
    ] {
        if read_from(&compartments, &guid).is_some_and(|value| value != 0) {
            return false;
        }
    }
    true
}

/// いまの入力モードを掲示する。
///
/// 入切には触れない。そちらは利用者とアプリのもので、こちらが勝手に
/// 入れてよいものではない (ADR-0012)。
///
/// 伝えられなくても入力そのものは続く。失敗しても記録するだけにする。
pub fn publish_mode(thread_manager: &ITfThreadMgr, client_id: u32, mode: InputMode) {
    let Ok(compartments) = thread_manager.cast::<ITfCompartmentMgr>() else {
        log::write("区画を扱えない");
        return;
    };

    // 文の変換の仕方。SKK は文法解析をしないが、掲示しないと
    // 「変換方式が決まっていない」扱いになる。
    write(
        &compartments,
        client_id,
        &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE,
        TF_SENTENCEMODE_PHRASEPREDICT,
    );

    write(
        &compartments,
        client_id,
        &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
        conversion_mode(mode),
    );
}

/// 入切の区画の変化を知らせてもらう。返るのは外すための受付番号。
pub fn advise_open_close(thread_manager: &ITfThreadMgr, sink: &IUnknown) -> Option<u32> {
    let compartments = thread_manager.cast::<ITfCompartmentMgr>().ok()?;
    // SAFETY: GUID は定数、受け口はこちらが生かし続けるもの。
    unsafe {
        let compartment = compartments
            .GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
            .ok()?;
        let source = compartment.cast::<ITfSource>().ok()?;
        // 渡すのは**受け口の種類**であって、見張る区画の GUID ではない。
        // 区画はもう `GetCompartment` で選んである。
        source.AdviseSink(&ITfCompartmentEventSink::IID, sink).ok()
    }
}

/// 入切の区画の通知を止める。
pub fn unadvise_open_close(thread_manager: &ITfThreadMgr, cookie: u32) {
    let Ok(compartments) = thread_manager.cast::<ITfCompartmentMgr>() else {
        return;
    };
    // SAFETY: 受付番号は [`advise_open_close`] が返したもの。
    unsafe {
        let Ok(compartment) = compartments.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
        else {
            return;
        };
        if let Ok(source) = compartment.cast::<ITfSource>() {
            let _ = source.UnadviseSink(cookie);
        }
    }
}

/// 入力モードを、Windows が使う語彙に訳す。
///
/// 日本語入力の慣例に従う。`ALPHANUMERIC` は 0 なので、英数は
/// 「何も立っていない」状態として表される。
pub fn conversion_mode(mode: InputMode) -> u32 {
    match mode {
        // かな入力。全角で、ローマ字から作る。
        InputMode::Hiragana => {
            TF_CONVERSIONMODE_NATIVE | TF_CONVERSIONMODE_FULLSHAPE | TF_CONVERSIONMODE_ROMAN
        }
        InputMode::Katakana => {
            TF_CONVERSIONMODE_NATIVE
                | TF_CONVERSIONMODE_FULLSHAPE
                | TF_CONVERSIONMODE_ROMAN
                | TF_CONVERSIONMODE_KATAKANA
        }
        // 半角カタカナは全角の印を落とす。
        InputMode::HalfKatakana => {
            TF_CONVERSIONMODE_NATIVE | TF_CONVERSIONMODE_ROMAN | TF_CONVERSIONMODE_KATAKANA
        }
        InputMode::FullAscii => TF_CONVERSIONMODE_ALPHANUMERIC | TF_CONVERSIONMODE_FULLSHAPE,
        InputMode::Ascii => TF_CONVERSIONMODE_ALPHANUMERIC,
    }
}

/// スレッドの区画から値を一つ読む。
fn read(thread_manager: &ITfThreadMgr, guid: &GUID) -> Option<i32> {
    let compartments = thread_manager.cast::<ITfCompartmentMgr>().ok()?;
    read_from(&compartments, guid)
}

/// 区画から整数を読む。入っていなければ `None`。
fn read_from(compartments: &ITfCompartmentMgr, guid: &GUID) -> Option<i32> {
    // SAFETY: GUID は定数で、読んだ値はこの関数の中で使い切る。
    unsafe {
        let compartment = compartments.GetCompartment(guid).ok()?;
        let variant = compartment.GetValue().ok()?;
        let inner = &variant.Anonymous.Anonymous;
        // 空の区画は「まだ誰も書いていない」。既定の値と混同しない。
        (inner.vt == VT_I4).then(|| inner.Anonymous.lVal)
    }
}

/// 区画に値を一つ書く。
fn write(compartments: &ITfCompartmentMgr, client_id: u32, guid: &GUID, value: u32) {
    // SAFETY: GUID は定数で、書き込む値はこの呼び出しの間だけ使われる。
    unsafe {
        let Ok(compartment) = compartments.GetCompartment(guid) else {
            return;
        };
        let variant = integer(value as i32);
        if let Err(e) = compartment.SetValue(client_id, &variant) {
            log::write(&format!("区画に書けなかった: {}", e.message()));
        }
    }
}

/// 整数を入れた値を作る。
///
/// `VARIANT` は共用体の入れ子なので、組み立ててから包む。中の欄へ
/// 直接書こうとすると、古い値の後始末が走る扱いになって書けない。
fn integer(value: i32) -> VARIANT {
    let inner = VARIANT_0_0 {
        vt: VT_I4,
        wReserved1: 0,
        wReserved2: 0,
        wReserved3: 0,
        Anonymous: VARIANT_0_0_0 { lVal: value },
    };
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(inner),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kana_modes_are_native_and_roman() {
        for mode in [
            InputMode::Hiragana,
            InputMode::Katakana,
            InputMode::HalfKatakana,
        ] {
            let flags = conversion_mode(mode);
            assert_ne!(flags & TF_CONVERSIONMODE_NATIVE, 0, "{mode:?}");
            assert_ne!(flags & TF_CONVERSIONMODE_ROMAN, 0, "{mode:?}");
        }
    }

    #[test]
    fn katakana_modes_say_so() {
        assert_ne!(
            conversion_mode(InputMode::Katakana) & TF_CONVERSIONMODE_KATAKANA,
            0
        );
        assert_ne!(
            conversion_mode(InputMode::HalfKatakana) & TF_CONVERSIONMODE_KATAKANA,
            0
        );
        assert_eq!(
            conversion_mode(InputMode::Hiragana) & TF_CONVERSIONMODE_KATAKANA,
            0
        );
    }

    #[test]
    fn half_width_drops_the_full_shape_flag() {
        assert_ne!(
            conversion_mode(InputMode::Katakana) & TF_CONVERSIONMODE_FULLSHAPE,
            0
        );
        assert_eq!(
            conversion_mode(InputMode::HalfKatakana) & TF_CONVERSIONMODE_FULLSHAPE,
            0
        );
        assert_eq!(
            conversion_mode(InputMode::Ascii) & TF_CONVERSIONMODE_FULLSHAPE,
            0
        );
        assert_ne!(
            conversion_mode(InputMode::FullAscii) & TF_CONVERSIONMODE_FULLSHAPE,
            0
        );
    }

    #[test]
    fn ascii_is_the_empty_state() {
        assert_eq!(conversion_mode(InputMode::Ascii), 0);
    }

    #[test]
    fn every_mode_maps_to_something_distinct() {
        let modes = [
            InputMode::Hiragana,
            InputMode::Katakana,
            InputMode::HalfKatakana,
            InputMode::FullAscii,
            InputMode::Ascii,
        ];
        let mut seen: Vec<u32> = modes.iter().map(|m| conversion_mode(*m)).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "モードごとに別の値になる");
    }
}
