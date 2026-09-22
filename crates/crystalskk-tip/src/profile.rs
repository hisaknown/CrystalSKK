//! 入力方式としての登録。
//!
//! COM のクラス登録 ([`crate::registry`]) だけでは、TSF は CrystalSKK を
//! 知らない。入力方式の一覧に加え、キーボード系の TIP だと名乗って初めて
//! 言語バーに現れる。

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, GUID_TFCAT_TIP_KEYBOARD, ITfCategoryMgr,
    ITfInputProcessorProfileMgr,
};
use windows::core::Result;

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

        // キーボードから入力する TIP だと名乗る。これがないと言語バーに出ない。
        let categories: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        categories.RegisterCategory(
            &CLSID_CRYSTALSKK,
            &GUID_TFCAT_TIP_KEYBOARD,
            &CLSID_CRYSTALSKK,
        )?;
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
            let _ = categories.UnregisterCategory(
                &CLSID_CRYSTALSKK,
                &GUID_TFCAT_TIP_KEYBOARD,
                &CLSID_CRYSTALSKK,
            );
        }

        let profiles: ITfInputProcessorProfileMgr =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        profiles.UnregisterProfile(&CLSID_CRYSTALSKK, LANGID_JA_JP, &GUID_CRYSTALSKK_PROFILE, 0)?;
    }
    Ok(())
}

/// DLL 内のアイコンの位置。まだ用意していないので既定のものが使われる。
const ICON_INDEX: u32 = 0;

/// 終端の NUL を含まない UTF-16 列。`RegisterProfile` は長さで受け取る。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}
