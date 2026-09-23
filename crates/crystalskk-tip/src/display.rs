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
//! # 線は必ず自分で引く
//!
//! `bAttr` は部分の役目をアプリに伝える欄で、`TARGET_CONVERTED` は多くの
//! アプリで**選択中の塊**として塗られる。
//!
//! ただし**塗らないアプリもある**。一度そこを当てにして候補の線を省き、
//! 何も引かれない状態になった。自分で引いたうえで `bAttr` でも伝える —
//! **アプリの善意を当てにしない。**
//!
//! # 太さで段階を示し、模様で境を示す
//!
//! | 部分 | 線 |
//! |---|---|
//! | 見出し語 | 点線 |
//! | 送り仮名 | 破線 |
//! | 候補 | **太い実線** |
//!
//! 「まだ打っている → 決まった → 選んでいる」が、そのまま線の強さになる。
//! **注目しているところが太い**のは既存の日本語入力と同じ約束である。
//!
//! ただし太さだけでは、隣り合ったときの境が見えない。`▼送り` は候補と
//! 送り仮名が地続きなので、実線同士だと**一本の線に見える**。線と線の間に
//! 隙間は空けられない — 渡せるのは種類・太さ・色だけで、**描くのはアプリ**
//! である。
//!
//! そこで送り仮名は模様を変える。太い実線と破線なら、繋がっていても境が
//! 分かる。見出し語との境は `*` が受け持つので、点線と破線の差が細かくても
//! 困らない。
//!
//! 波線は使わない。綴り間違いの印として定着しているので、入力中の文字に
//! 使うと「間違っている」と読まれる。
//!
//! # 印は出さず、区切りだけ残す
//!
//! `▽` `▼` `*` は文書に出さない (PRD Q-09)。**状態は下線で分かる**ので、
//! 記号まで並べると文字列が読みにくくなる。
//!
//! ただし二つは役目が違う。
//!
//! - **`▽` `▼` は状態を表す。** 下線が同じことを言っているので、何も
//!   出さない。空白を置くと、書いている文が一文字ぶん右へずれる
//! - **`*` は境を示す。** 下線の点線と破線は見分けが付きにくいので、
//!   空白で区切りを見せる
//!
//! **CLI は記号のまま出す。** ターミナルに下線を引けないので、記号が唯一の
//! 手がかりになる。同じ `Preedit` から違う見せ方をしているだけである。

use windows::Win32::UI::TextServices::{
    IEnumTfDisplayAttributeInfo, IEnumTfDisplayAttributeInfo_Impl, ITfDisplayAttributeInfo,
    ITfDisplayAttributeInfo_Impl, TF_ATTR_INPUT, TF_ATTR_TARGET_CONVERTED, TF_DA_COLOR,
    TF_DISPLAYATTRIBUTE, TF_LS_DASH, TF_LS_DOT, TF_LS_NONE, TF_LS_SOLID,
};
use windows::core::{BSTR, ComObject, GUID, Result, implement};

use crystalskk_core::engine::{Role, Segment};

use crate::guard::guard;
use crate::guids::{
    GUID_DISPLAY_ATTRIBUTE_COMPLETION, GUID_DISPLAY_ATTRIBUTE_CONVERTED,
    GUID_DISPLAY_ATTRIBUTE_INPUT, GUID_DISPLAY_ATTRIBUTE_OKURI,
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
    /// 太く引くか。
    bold: bool,
    /// この部分は何か。
    kind: i32,
}

/// 見出し語。まだ打っている最中なので点線。
pub const INPUT: Attribute = Attribute {
    guid: GUID_DISPLAY_ATTRIBUTE_INPUT,
    description: "CrystalSKK: 見出し語",
    line: TF_LS_DOT.0,
    bold: false,
    kind: TF_ATTR_INPUT.0,
};

/// 送り仮名。決まっているので細い線。
///
/// **模様を候補と変えてある。** 実線にすると、`▼送り` で候補の太い実線と
/// 地続きになり、一本の線に見える。線の間に隙間は空けられないので、
/// 境は模様で示すしかない。
pub const OKURI: Attribute = Attribute {
    guid: GUID_DISPLAY_ATTRIBUTE_OKURI,
    description: "CrystalSKK: 送り仮名",
    line: TF_LS_DASH.0,
    bold: false,
    kind: TF_ATTR_INPUT.0,
};

/// 選ばれている候補。いま注目しているところなので太い実線。
///
/// 塗ってくれるアプリには `bAttr` で伝わる。**塗らないアプリでも線は
/// 残る。**
pub const CONVERTED: Attribute = Attribute {
    guid: GUID_DISPLAY_ATTRIBUTE_CONVERTED,
    description: "CrystalSKK: 変換中",
    line: TF_LS_SOLID.0,
    bold: true,
    kind: TF_ATTR_TARGET_CONVERTED.0,
};

/// 補完の当て推量。**線を引かない。**
///
/// 下線は打った文字のところで終わる。その先に線が無ければ、**まだ自分の
/// 文字ではない**と見て分かる。
///
/// 色を使えれば薄く出すところだが、色は決めない方針なので (暗い配色を
/// 追いかけることになる)、線の有無で示す。
pub const COMPLETION: Attribute = Attribute {
    guid: GUID_DISPLAY_ATTRIBUTE_COMPLETION,
    description: "CrystalSKK: 補完",
    line: TF_LS_NONE.0,
    bold: false,
    kind: TF_ATTR_INPUT.0,
};

/// 名乗るものすべて。
pub const ALL: &[Attribute] = &[INPUT, OKURI, CONVERTED, COMPLETION];

/// 区切りの役目に対する見え方。
///
/// 印と区切りは**続く部分に合わせる**。そこだけ違う線になると、一つの
/// 塊が途中で切れて見える。合わせる相手は呼ぶ側が渡す。
pub fn for_role(role: Role) -> Attribute {
    match role {
        Role::Midashi => INPUT,
        Role::Okuri => OKURI,
        Role::Completion => COMPLETION,
        // 単独で渡されたときは変換中として扱う。続く部分があれば、
        // そちらに合わせて上書きされる。
        Role::Candidate | Role::Marker | Role::Separator => CONVERTED,
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
            fBoldLine: self.bold.into(),
            crLine: TF_DA_COLOR::default(),
            bAttr: windows::Win32::UI::TextServices::TF_DA_ATTR_INFO(self.kind),
        }
    }
}

/// 状態の印 (`▽` `▼`) を文書に出すときの文字。
///
/// 出さない。下線が同じことを言っている。
pub const MARKER_TEXT: &str = "";

/// 区切り (`*`) を文書に出すときの文字。
///
/// 空白を置く。**下線の模様だけでは、見出し語と送り仮名の境が見分け
/// にくい。**
pub const SEPARATOR_TEXT: &str = " ";

/// 未確定の表示を、文書へ書く形に組み立てる。
///
/// 区切りごとに「貼る番号」と「出す文字」の組にする。ここで決まるのは
/// 二つ。
///
/// - **印は空白に置き換える。** 記号は出さない
/// - **印の見え方は続く部分に合わせる。** 印だけ違う線になると、一つの
///   塊が途中で切れて見える
pub fn document_segments(segments: &[Segment], atoms: Option<Atoms>) -> Vec<(u32, String)> {
    segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let role = match segment.role {
                Role::Marker | Role::Separator => segments
                    .get(index + 1)
                    .map_or(segment.role, |next| next.role),
                role => role,
            };
            let atom = atoms.map_or(0, |atoms| atoms.for_role(role));
            let text = match segment.role {
                Role::Marker => MARKER_TEXT.to_owned(),
                Role::Separator => SEPARATOR_TEXT.to_owned(),
                _ => segment.text.clone(),
            };
            (atom, text)
        })
        .collect()
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
    completion: u32,
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
                completion: categories.RegisterGUID(&COMPLETION.guid)?,
            })
        }
    }

    /// 役目に対する番号。
    pub fn for_role(&self, role: Role) -> u32 {
        match for_role(role).guid {
            g if g == INPUT.guid => self.input,
            g if g == OKURI.guid => self.okuri,
            g if g == COMPLETION.guid => self.completion,
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
    fn every_part_the_user_typed_gets_a_line() {
        // **アプリが塗ってくれることを当てにしない。** 一度それで、候補に
        // 何も引かれない状態になった。
        //
        // 補完の当て推量だけは別で、線が無いことが「まだ打っていない」の
        // 印になる。
        for attribute in ALL.iter().filter(|a| a.guid != COMPLETION.guid) {
            assert_ne!(attribute.line, TF_LS_NONE.0, "{}", attribute.description);
        }
    }

    #[test]
    fn the_guess_has_no_line_under_it() {
        // 下線が打った文字のところで終わる。その先は自分の文字ではない。
        assert_eq!(COMPLETION.line, TF_LS_NONE.0);
    }

    #[test]
    fn the_candidate_is_the_boldest() {
        // 注目しているところが一番強く出る。既存の日本語入力と同じ約束。
        let bold: Vec<&str> = ALL
            .iter()
            .filter(|a| a.bold)
            .map(|a| a.description)
            .collect();
        assert_eq!(bold, vec![CONVERTED.description], "太いのは候補だけ");
        assert_eq!(CONVERTED.kind, TF_ATTR_TARGET_CONVERTED.0);
    }

    #[test]
    fn the_okuri_looks_settled_and_the_midashi_does_not() {
        // 打っている最中と、決まっているところを見分けられるようにする。
        assert_ne!(INPUT.line, OKURI.line);
    }

    #[test]
    fn the_candidate_and_the_okuri_do_not_blur_into_one_line() {
        // `▼送り` では候補と送り仮名が地続きになる。**線の間に隙間は
        // 空けられない**ので、模様が同じだと一本に見える。
        assert_ne!(CONVERTED.line, OKURI.line, "隣り合う二つは模様を変える");
    }

    #[test]
    fn nothing_is_squiggled() {
        // 波線は綴り間違いの印として定着している。入力中の文字に使うと
        // 「間違っている」と読まれる。
        for attribute in ALL {
            assert_ne!(
                attribute.line,
                windows::Win32::UI::TextServices::TF_LS_SQUIGGLE.0,
                "{}",
                attribute.description
            );
        }
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

    fn segment(role: Role, text: &str) -> Segment {
        Segment {
            role,
            text: text.to_owned(),
        }
    }

    #[test]
    fn the_markers_leave_and_the_separator_becomes_a_space() {
        // `▽おく*り` は `おく り` になる。**状態の印は消し、境だけ残す。**
        let written = document_segments(
            &[
                segment(Role::Marker, "▽"),
                segment(Role::Midashi, "おく"),
                segment(Role::Separator, "*"),
                segment(Role::Okuri, "り"),
            ],
            None,
        );
        let text: String = written.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(text, "おく り");
    }

    #[test]
    fn the_text_does_not_shift_when_conversion_starts() {
        // 印に空白を置くと、書いている文が一文字ぶん右へずれる。
        let written = document_segments(&[segment(Role::Marker, "▽")], None);
        let text: String = written.iter().map(|(_, t)| t.as_str()).collect();
        assert!(text.is_empty(), "印は場所を取らない");
    }

    #[test]
    fn a_marker_takes_the_look_of_what_follows_it() {
        // 印だけ違う線になると、一つの塊が途中で切れて見える。
        let atoms = Atoms {
            input: 1,
            okuri: 2,
            converted: 3,
            completion: 4,
        };
        let written = document_segments(
            &[segment(Role::Separator, "*"), segment(Role::Okuri, "り")],
            Some(atoms),
        );
        assert_eq!(written[0].0, written[1].0, "区切りは送り仮名に合わせる");
    }

    #[test]
    fn a_trailing_marker_keeps_its_own_look() {
        // 続く部分が無いときは、自分の役目のまま。**落ちたりしない。**
        let written = document_segments(&[segment(Role::Marker, "▼")], None);
        assert_eq!(written.len(), 1);
    }

    #[test]
    fn every_role_has_a_look() {
        for role in [
            Role::Marker,
            Role::Separator,
            Role::Midashi,
            Role::Okuri,
            Role::Candidate,
            Role::Completion,
        ] {
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
