//! 入力モードを Windows へ伝える。
//!
//! 言語バーの項目に絵を返すだけでは、トレイの表示は出ない。**いまどの
//! 入力モードにいるかは「区画」(compartment) に書いて伝える**のが TSF の
//! 決まりで、Windows の表示はそちらを見ている。
//!
//! 区画は入力方式と外の世界が値をやり取りする掲示板のようなもので、
//! `GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION` に書いた値が
//! 「ひらがな」「全角カタカナ」…… として解釈される。
//!
//! 値の組み合わせは日本語入力の慣例に従う。CrystalSKK の五つのモードを
//! その語彙へ訳すのが [`conversion_mode`] である。

use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};
use windows::Win32::UI::TextServices::{
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION, GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE,
    ITfCompartmentMgr, ITfThreadMgr, TF_CONVERSIONMODE_ALPHANUMERIC, TF_CONVERSIONMODE_FULLSHAPE,
    TF_CONVERSIONMODE_KATAKANA, TF_CONVERSIONMODE_NATIVE, TF_CONVERSIONMODE_ROMAN,
    TF_SENTENCEMODE_PHRASEPREDICT,
};
use windows::core::{GUID, Interface};

use crystalskk_core::InputMode;

use crate::log;

/// いまの入力モードを掲示する。
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
