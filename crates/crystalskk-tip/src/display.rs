//! 未確定の文字の見え方。
//!
//! 未確定の文字列には、アプリが既定の下線を引く。それだけでは**どこまでが
//! 見出し語で、どこからが送り仮名か**が分からない。SKK は一つの未確定の
//! 中に役目の違う部分を並べるので、その差が要る。
//!
//! # 色は決めない
//!
//! 与えるのは線の種類と「ここは何か」だけで、色は指定しない。**アプリの
//! 配色に任せれば、暗い背景でも勝手に成立する。** こちらで色を決めると、
//! 配色が変わるたびに破綻を追いかけることになる。CorvusSKK の既定も色を
//! 指定していない。
//!
//! # 効くのは線よりも「ここは何か」
//!
//! `bAttr` は部分の役目をアプリに伝える欄で、アプリは自分の流儀で強調
//! する。`TARGET_CONVERTED` は多くのアプリで**選択中の塊**として塗られる。
//! だから候補の部分は線を引かない。アプリが塗るので要らない。

use windows::Win32::UI::TextServices::{
    IEnumTfDisplayAttributeInfo, IEnumTfDisplayAttributeInfo_Impl, ITfDisplayAttributeInfo,
    ITfDisplayAttributeInfo_Impl, TF_ATTR_INPUT, TF_ATTR_TARGET_CONVERTED, TF_DA_COLOR,
    TF_DISPLAYATTRIBUTE, TF_LS_DOT, TF_LS_NONE, TF_LS_SOLID,
};
use windows::core::{BSTR, ComObject, GUID, Result, implement};

use crystalskk_core::engine::Role;

use crate::guard::guard;
use crate::guids::{
    GUID_DISPLAY_ATTRIBUTE_CONVERTED, GUID_DISPLAY_ATTRIBUTE_INPUT, GUID_DISPLAY_ATTRIBUTE_OKURI,
};

/// 未確定の区切りに与える見え方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attribute {
    /// この見え方を指す GUID。
    pub guid: GUID,
    /// 設定画面などに出す名前。
    pub description: &'static str,
    /// 線の種類。
    line: i32,
    /// この部分は何か。
    kind: i32,
}

/// 見出し語。打っている最中なので点線。
pub const INPUT: Attribute = Attribute {
    guid: GUID_DISPLAY_ATTRIBUTE_INPUT,
    description: "CrystalSKK: 見出し語",
    line: TF_LS_DOT.0,
    kind: TF_ATTR_INPUT.0,
};

/// 送り仮名。ここは決まっているので実線。
pub const OKURI: Attribute = Attribute {
    guid: GUID_DISPLAY_ATTRIBUTE_OKURI,
    description: "CrystalSKK: 送り仮名",
    line: TF_LS_SOLID.0,
    kind: TF_ATTR_INPUT.0,
};

/// 選ばれている候補。線は引かず、アプリに塗らせる。
pub const CONVERTED: Attribute = Attribute {
    guid: GUID_DISPLAY_ATTRIBUTE_CONVERTED,
    description: "CrystalSKK: 変換中",
    line: TF_LS_NONE.0,
    kind: TF_ATTR_TARGET_CONVERTED.0,
};

/// 名乗るものすべて。
pub const ALL: &[Attribute] = &[INPUT, OKURI, CONVERTED];

/// 区切りの役目に対する見え方。
///
/// 印 (`▽` `▼`) は隣の部分と同じ扱いにする。**別扱いにする利点が、いまは
/// 無い。** 分けたくなったら足せる。
pub fn for_role(role: Role) -> Attribute {
    match role {
        Role::Midashi => INPUT,
        Role::Okuri => OKURI,
        Role::Candidate | Role::Marker => CONVERTED,
    }
}

impl Attribute {
    /// TSF へ渡す形。
    fn to_tsf(self) -> TF_DISPLAYATTRIBUTE {
        TF_DISPLAYATTRIBUTE {
            // 色は決めない。アプリの配色に任せる。
            crText: TF_DA_COLOR::default(),
            crBk: TF_DA_COLOR::default(),
            lsStyle: windows::Win32::UI::TextServices::TF_DA_LINESTYLE(self.line),
            fBoldLine: false.into(),
            crLine: TF_DA_COLOR::default(),
            bAttr: windows::Win32::UI::TextServices::TF_DA_ATTR_INFO(self.kind),
        }
    }
}

/// 見え方に振られた番号。
///
/// 文書の範囲へ貼るときは GUID ではなくこの番号を使う。TSF に登録すると
/// 貰える。**スレッドごとに一度取れば足りる。**
#[derive(Debug, Clone, Copy)]
pub struct Atoms {
    input: u32,
    okuri: u32,
    converted: u32,
}

impl Atoms {
    /// 見え方を登録して番号を貰う。
    pub fn register() -> windows::core::Result<Self> {
        // SAFETY: COM は呼び出し側で初期化済み。生成した参照はここで完結する。
        let categories: windows::Win32::UI::TextServices::ITfCategoryMgr = unsafe {
            windows::Win32::System::Com::CoCreateInstance(
                &windows::Win32::UI::TextServices::CLSID_TF_CategoryMgr,
                None,
                windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
            )?
        };
        // SAFETY: GUID は定数。
        unsafe {
            Ok(Self {
                input: categories.RegisterGUID(&INPUT.guid)?,
                okuri: categories.RegisterGUID(&OKURI.guid)?,
                converted: categories.RegisterGUID(&CONVERTED.guid)?,
            })
        }
    }

    /// 役目に対する番号。
    pub fn for_role(&self, role: Role) -> u32 {
        match for_role(role).guid {
            g if g == INPUT.guid => self.input,
            g if g == OKURI.guid => self.okuri,
            _ => self.converted,
        }
    }
}

/// 一つの見え方を TSF へ差し出す口。
#[implement(ITfDisplayAttributeInfo)]
pub struct AttributeInfo {
    attribute: Attribute,
}

impl std::fmt::Debug for AttributeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttributeInfo")
            .field("description", &self.attribute.description)
            .finish_non_exhaustive()
    }
}

impl AttributeInfo {
    pub fn new(attribute: Attribute) -> Self {
        Self { attribute }
    }
}

impl ITfDisplayAttributeInfo_Impl for AttributeInfo_Impl {
    fn GetGUID(&self) -> Result<GUID> {
        guard("GetGUID(display)", || Ok(self.this.attribute.guid))
    }

    fn GetDescription(&self) -> Result<BSTR> {
        guard("GetDescription(display)", || {
            Ok(BSTR::from(self.this.attribute.description))
        })
    }

    // TSF が決めた形なので、生のポインタを受けるしかない。中では
    // 確かめてから使う。
    #[allow(
        clippy::not_unsafe_ptr_arg_deref,
        reason = "COM の口の形が決まっている"
    )]
    fn GetAttributeInfo(&self, pda: *mut TF_DISPLAYATTRIBUTE) -> Result<()> {
        guard("GetAttributeInfo", || {
            if pda.is_null() {
                return Err(windows::Win32::Foundation::E_INVALIDARG.into());
            }
            // SAFETY: null でないことを確かめた書き込み先へ写す。
            unsafe { *pda = self.this.attribute.to_tsf() };
            Ok(())
        })
    }

    /// 設定画面から変えられてもよいが、いまは受け付けない。
    fn SetAttributeInfo(&self, _pda: *const TF_DISPLAYATTRIBUTE) -> Result<()> {
        guard("SetAttributeInfo", || {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        })
    }

    /// 変えていないので、戻すものもない。
    fn Reset(&self) -> Result<()> {
        guard("Reset(display)", || Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_attribute_has_its_own_guid() {
        let mut seen: Vec<u128> = ALL.iter().map(|a| a.guid.to_u128()).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "見え方ごとに別の GUID");
    }

    #[test]
    fn the_candidate_is_left_for_the_application_to_paint() {
        // 線を引くと、アプリが塗ったうえに下線が重なる。
        assert_eq!(CONVERTED.line, TF_LS_NONE.0);
        assert_eq!(CONVERTED.kind, TF_ATTR_TARGET_CONVERTED.0);
    }

    #[test]
    fn the_okuri_looks_settled_and_the_midashi_does_not() {
        // 打っている最中と、決まっているところを見分けられるようにする。
        assert_ne!(INPUT.line, OKURI.line);
    }

    #[test]
    fn no_attribute_picks_a_colour() {
        // 色を決めると、暗い配色のたびに破綻を追いかけることになる。
        for attribute in ALL {
            let tsf = attribute.to_tsf();
            assert_eq!(tsf.crText.r#type.0, 0, "{}", attribute.description);
            assert_eq!(tsf.crBk.r#type.0, 0, "{}", attribute.description);
        }
    }

    #[test]
    fn every_role_has_a_look() {
        for role in [Role::Marker, Role::Midashi, Role::Okuri, Role::Candidate] {
            let attribute = for_role(role);
            assert!(ALL.contains(&attribute), "{role:?}");
        }
    }
}

/// 名乗った見え方を順に返す係。
///
/// アプリは「どんな見え方があるか」をまずこれで尋ねる。**貼った番号の
/// 意味を知るのに要る。**
#[implement(IEnumTfDisplayAttributeInfo)]
#[derive(Default)]
pub struct AttributeEnum {
    /// 次に返す位置。
    next: std::cell::Cell<usize>,
}

impl std::fmt::Debug for AttributeEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttributeEnum")
            .field("next", &self.next.get())
            .finish()
    }
}

impl IEnumTfDisplayAttributeInfo_Impl for AttributeEnum_Impl {
    fn Clone(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        guard("Clone(display)", || {
            let copy = ComObject::new(AttributeEnum::default());
            copy.next.set(self.this.next.get());
            Ok(copy.to_interface())
        })
    }

    #[allow(
        clippy::not_unsafe_ptr_arg_deref,
        reason = "COM の口の形が決まっている"
    )]
    fn Next(
        &self,
        ulcount: u32,
        rginfo: *mut Option<ITfDisplayAttributeInfo>,
        pcfetched: *mut u32,
    ) -> Result<()> {
        guard("Next(display)", || {
            if rginfo.is_null() {
                return Err(windows::Win32::Foundation::E_INVALIDARG.into());
            }
            let mut fetched = 0u32;
            for slot in 0..ulcount as usize {
                let Some(attribute) = ALL.get(self.this.next.get()) else {
                    break;
                };
                self.this.next.set(self.this.next.get() + 1);

                let info = ComObject::new(AttributeInfo::new(*attribute));
                // SAFETY: 渡された配列の、告げられた長さの内側だけに書く。
                unsafe { rginfo.add(slot).write(Some(info.to_interface())) };
                fetched += 1;
            }
            if !pcfetched.is_null() {
                // SAFETY: null でないことを確かめた書き込み先へ写す。
                unsafe { *pcfetched = fetched };
            }
            // 頼まれた数に届かなければ、終わりという意味の S_FALSE。
            if fetched < ulcount {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::S_FALSE,
                ));
            }
            Ok(())
        })
    }

    fn Reset(&self) -> Result<()> {
        guard("Reset(enum)", || {
            self.this.next.set(0);
            Ok(())
        })
    }

    fn Skip(&self, ulcount: u32) -> Result<()> {
        guard("Skip(display)", || {
            let next = self.this.next.get().saturating_add(ulcount as usize);
            self.this.next.set(next.min(ALL.len()));
            Ok(())
        })
    }
}

/// GUID から見え方を探す。
pub fn by_guid(guid: &GUID) -> Option<Attribute> {
    ALL.iter().find(|a| a.guid == *guid).copied()
}
