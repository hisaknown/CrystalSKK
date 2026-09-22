//! 入力方式としての登録。
//!
//! COM のクラス登録 ([`crate::registry`]) だけでは、TSF は CrystalSKK を
//! 知らない。入力方式の一覧に加え、キーボード系の TIP だと名乗って初めて
//! 言語バーに現れる。
//!
//! # 「どんな場面で使えるか」も名乗る
//!
//! TIP は分類 (category) を登録して、自分に何ができるかを宣言する。
//! 名乗らない能力は**無いものとして扱われる**。
//!
//! これを一つしか登録していなかったために、二つの症状が出ていた。
//! トレイにモードが出ず、スタートメニューの検索欄では入力方式として
//! 選ぶことすらできなかった (ADR-0011)。

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, GUID_TFCAT_TIP_KEYBOARD,
    GUID_TFCAT_TIPCAP_COMLESS, GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT, GUID_TFCAT_TIPCAP_SECUREMODE,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT, GUID_TFCAT_TIPCAP_UIELEMENTENABLED, ITfCategoryMgr,
    ITfInputProcessorProfileMgr,
};
use windows::core::{GUID, Result};

use crate::guids::{CLSID_CRYSTALSKK, GUID_CRYSTALSKK_PROFILE, LANGID_JA_JP, PROFILE_DESCRIPTION};

/// 入力方式として登録する。
///
/// `icon_path` には DLL の場所を渡す。TSF はそこからアイコンを取り出す。
pub fn register_profile(icon_path: &str) -> Result<()> {
    // SAFETY: COM は呼び出し側で初期化済み。生成した参照はこの関数の中で完結する。
    unsafe {
        let profiles: ITfInputProcessorProfileMgr =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;

        let description: Vec<u16> = wide(PROFILE_DESCRIPTION);
        let icon: Vec<u16> = wide(icon_path);

        profiles.RegisterProfile(
            &CLSID_CRYSTALSKK,
            LANGID_JA_JP,
            &GUID_CRYSTALSKK_PROFILE,
            &description,
            &icon,
            ICON_INDEX,
            HKL::default(),
            0,
            // 既定の入力方式にはしない。利用者が選ぶまで邪魔をしない。
            false,
            0,
        )?;

        let categories: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        for category in CATEGORIES {
            categories.RegisterCategory(&CLSID_CRYSTALSKK, category, &CLSID_CRYSTALSKK)?;
        }
    }
    Ok(())
}

/// 入力方式の登録を消す。
pub fn unregister_profile() -> Result<()> {
    // SAFETY: 上と同じ。失敗しても後続の後始末は進める。
    unsafe {
        if let Ok(categories) =
            CoCreateInstance::<_, ITfCategoryMgr>(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)
        {
            for category in CATEGORIES {
                let _ =
                    categories.UnregisterCategory(&CLSID_CRYSTALSKK, category, &CLSID_CRYSTALSKK);
            }
        }

        let profiles: ITfInputProcessorProfileMgr =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        profiles.UnregisterProfile(&CLSID_CRYSTALSKK, LANGID_JA_JP, &GUID_CRYSTALSKK_PROFILE, 0)?;
    }
    Ok(())
}

/// DLL 内のアイコンの位置。まだ用意していないので既定のものが使われる。
const ICON_INDEX: u32 = 0;

/// 名乗る分類。
///
/// ここに無い能力は「持っていない」と見なされる。表示が出ない、
/// 選べない、といった症状の多くはここの漏れで説明がつく。
///
/// 表示属性 (下線) の分類は、実装してから足す。できないことを
/// 名乗っても仕方がない。
const CATEGORIES: &[GUID] = &[
    // キーボードから入力する TIP である。これが無いと言語バーに出ない。
    GUID_TFCAT_TIP_KEYBOARD,
    // ログオン画面のような、安全が要る場面でも動ける。
    GUID_TFCAT_TIPCAP_SECUREMODE,
    // 候補一覧などの UI を、システム側に扱わせられる。
    GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
    // 入力モードを区画で伝える (ADR-0010)。トレイの表示はこれを見る。
    GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT,
    // COM の登録に頼らず読み込める。制限の強い場面で要る。
    GUID_TFCAT_TIPCAP_COMLESS,
    // ストアアプリやスタートメニューのような、隔離された場面で動ける。
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    // トレイの入力表示に対応する。
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
];

/// 終端の NUL を含まない UTF-16 列。`RegisterProfile` は長さで受け取る。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}
