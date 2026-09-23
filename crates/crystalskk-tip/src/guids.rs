//! CrystalSKK が名乗る GUID。
//!
//! これらは一度公開したら**変えてはならない**。利用者のレジストリと
//! 入力方式の設定はこの値で CrystalSKK を指しているため、変えると
//! 「入力方式が消えた」ように見え、古い登録が残り続ける。

use windows::core::GUID;

/// TIP そのものを指す COM のクラス ID。
pub const CLSID_CRYSTALSKK: GUID = GUID::from_u128(0x5cd1c143_735e_4051_986e_69864a08febe);

/// 入力方式 (言語バーに並ぶ項目) を指す GUID。
pub const GUID_CRYSTALSKK_PROFILE: GUID = GUID::from_u128(0x3894d2cd_3ec7_4877_8fcd_f42c42c3eba3);

/// 言語バーに独自の項目を出すときの GUID。
///
/// 入力モードの表示には使わない。そちらは Windows が定める
/// `GUID_LBI_INPUTMODE` を名乗る必要がある。
#[allow(dead_code, reason = "独自の項目を足す段階で使う")]
pub const GUID_CRYSTALSKK_LANGBAR: GUID = GUID::from_u128(0x7d3f9a41_0c2e_45b8_9a6d_1f4c8e2b7a09);

/// 見出し語入力中の文字に付ける表示属性。
pub const GUID_DISPLAY_ATTRIBUTE_INPUT: GUID =
    GUID::from_u128(0xbfdd4dbc_77c0_4f2d_beae_4fe6aabbe510);

/// 候補選択中の文字に付ける表示属性。
pub const GUID_DISPLAY_ATTRIBUTE_CONVERTED: GUID =
    GUID::from_u128(0x9b6004e5_bb9b_4b85_9741_507b98ddd64d);

/// 動的補完の候補に付ける表示属性。
pub const GUID_DISPLAY_ATTRIBUTE_COMPLETION: GUID =
    GUID::from_u128(0xe7a1c0d4_5b92_4e38_9c07_2a6f13b84d5e);

/// 送り仮名に付ける表示属性。
pub const GUID_DISPLAY_ATTRIBUTE_OKURI: GUID =
    GUID::from_u128(0x4c0e5b71_8d3a_4f26_a915_b0d27e63c8fa);

/// 入力方式を入にする横取りキーの GUID。
pub const GUID_PRESERVED_KEY_ON: GUID = GUID::from_u128(0x0a1c4f62_6d8e_4b3a_9c57_2e0b8d4a71f3);

/// 入力方式を切にする横取りキーの GUID。
pub const GUID_PRESERVED_KEY_OFF: GUID = GUID::from_u128(0x5e8b2d09_47a1_4c6f_b3d2_8f1a6c05e2b4);

/// 候補一覧としてシステムへ差し出す口の GUID。
pub const GUID_CANDIDATE_LIST_ELEMENT: GUID =
    GUID::from_u128(0x2f7b8c14_9d63_4a05_b1e8_37c0a64df592);

/// 言語バーと設定画面に出る名前。
pub const PROFILE_DESCRIPTION: &str = "CrystalSKK";

/// COM のクラス登録に書く名前。
pub const CLASS_DESCRIPTION: &str = "CrystalSKK Text Input Processor";

/// 日本語 (日本)。TIP はこの言語に結び付けて登録する。
pub const LANGID_JA_JP: u16 = 0x0411;
