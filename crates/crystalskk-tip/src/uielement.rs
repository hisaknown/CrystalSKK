//! 候補一覧を「持っている」とシステムへ申告する。
//!
//! 自前の窓 ([`crate::candwin`]) は、普通のアプリでは十分に働く。しかし
//! ストアアプリのような場面では、**アプリが候補一覧を自分で描きたがる**。
//! そのとき要るのが、一覧の中身を渡すためのこの口である。
//!
//! # 申告すると、描くかどうかを向こうが決める
//!
//! 一覧を出すとき `BeginUIElement` で申告する。返ってくる答えが二通りある。
//!
//! - **描いてよい** … 自前の窓を出す。普通のアプリはこちら
//! - **こちらで描く** … 自前の窓は出さず、中身を渡すだけにする
//!
//! どちらであっても、一覧が変わるたびに `UpdateUIElement` で知らせ、
//! 終わったら `EndUIElement` で畳む。
//!
//! # 申告しないとどうなるか
//!
//! **その場面では候補が一切出ない。** 自前の窓は描けず、アプリも中身を
//! 知りようがないので、利用者からは「変換できるが候補が見えない」という
//! 状態になる。

use std::cell::RefCell;

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::UI::TextServices::{
    ITfCandidateListUIElement, ITfCandidateListUIElement_Impl, ITfDocumentMgr, ITfThreadMgr,
    ITfUIElement, ITfUIElement_Impl, ITfUIElementMgr, TF_CLUIE_COUNT, TF_CLUIE_CURRENTPAGE,
    TF_CLUIE_DOCUMENTMGR, TF_CLUIE_PAGEINDEX, TF_CLUIE_SELECTION, TF_CLUIE_STRING,
};
use windows::core::{BOOL, BSTR, ComObject, GUID, Interface, Result, implement};

use crate::guard::guard;
use crate::guids::GUID_CANDIDATE_LIST_ELEMENT;
use crate::log;

/// アプリへ渡す一覧の中身。
///
/// 一覧に載る候補だけを持つ。**載らない候補まで数に入れると、ページの
/// 区切りが合わなくなる。** 一つずつ見せている間は一覧そのものが無い。
#[derive(Debug, Default, Clone)]
pub struct ListSnapshot {
    /// 一覧に載る候補の表示文字列。
    pub items: Vec<String>,
    /// いま見せているページの先頭。
    pub selection: u32,
    /// 一ページに載る数。
    pub page_size: u32,
    /// いま何ページ目か。0 から数える。
    pub current_page: u32,
}

impl ListSnapshot {
    /// 各ページの先頭。
    fn page_starts(&self) -> Vec<u32> {
        if self.page_size == 0 {
            return Vec::new();
        }
        let count = u32::try_from(self.items.len()).unwrap_or(u32::MAX);
        (0..count).step_by(self.page_size as usize).collect()
    }
}

/// 候補一覧としてシステムへ差し出す口。
#[implement(ITfCandidateListUIElement, ITfUIElement)]
pub struct CandidateListElement {
    /// いまの中身。
    snapshot: RefCell<ListSnapshot>,
    /// 焦点のある文書。アプリが「どこに対する候補か」を知るのに使う。
    documents: RefCell<Option<ITfDocumentMgr>>,
    /// 自前の窓を出してよいか。アプリが自分で描くときは偽になる。
    ours_to_draw: RefCell<bool>,
}

impl std::fmt::Debug for CandidateListElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CandidateListElement")
            .field("items", &self.snapshot.borrow().items.len())
            .field("ours_to_draw", &self.ours_to_draw.borrow())
            .finish_non_exhaustive()
    }
}

impl Default for CandidateListElement {
    fn default() -> Self {
        Self::new()
    }
}

impl CandidateListElement {
    pub fn new() -> Self {
        Self {
            snapshot: RefCell::new(ListSnapshot::default()),
            documents: RefCell::new(None),
            ours_to_draw: RefCell::new(true),
        }
    }

    /// 中身を入れ替える。
    pub fn set(&self, snapshot: ListSnapshot, documents: Option<ITfDocumentMgr>) {
        *self.snapshot.borrow_mut() = snapshot;
        *self.documents.borrow_mut() = documents;
    }

    /// 自前の窓を出してよいか。
    pub fn ours_to_draw(&self) -> bool {
        *self.ours_to_draw.borrow()
    }

    fn set_ours_to_draw(&self, ours: bool) {
        *self.ours_to_draw.borrow_mut() = ours;
    }
}

impl ITfUIElement_Impl for CandidateListElement_Impl {
    fn GetDescription(&self) -> Result<BSTR> {
        guard("GetDescription", || Ok(BSTR::from("CrystalSKK の候補一覧")))
    }

    fn GetGUID(&self) -> Result<GUID> {
        guard("GetGUID", || Ok(GUID_CANDIDATE_LIST_ELEMENT))
    }

    /// アプリから「出せ」「隠せ」と言われた。
    ///
    /// 自分で描くと言ったアプリは、ここを呼んでこない。呼んでくるのは
    /// 自前の窓を使う場合だけである。
    fn Show(&self, bshow: BOOL) -> Result<()> {
        guard("Show", || {
            self.this.set_ours_to_draw(bshow.as_bool());
            Ok(())
        })
    }

    fn IsShown(&self) -> Result<BOOL> {
        guard("IsShown", || Ok(self.this.ours_to_draw().into()))
    }
}

impl ITfCandidateListUIElement_Impl for CandidateListElement_Impl {
    /// 前回から何が変わったか。
    ///
    /// 細かく分けても得がないので、**全部変わったことにする**。一覧は
    /// 一度に一ページぶんしか動かないので、読み直す手間は知れている。
    fn GetUpdatedFlags(&self) -> Result<u32> {
        guard("GetUpdatedFlags", || {
            Ok(TF_CLUIE_DOCUMENTMGR
                | TF_CLUIE_COUNT
                | TF_CLUIE_SELECTION
                | TF_CLUIE_STRING
                | TF_CLUIE_PAGEINDEX
                | TF_CLUIE_CURRENTPAGE)
        })
    }

    fn GetDocumentMgr(&self) -> Result<ITfDocumentMgr> {
        guard("GetDocumentMgr", || {
            self.this
                .documents
                .borrow()
                .clone()
                .ok_or_else(|| E_INVALIDARG.into())
        })
    }

    fn GetCount(&self) -> Result<u32> {
        guard("GetCount", || {
            Ok(u32::try_from(self.this.snapshot.borrow().items.len()).unwrap_or(0))
        })
    }

    fn GetSelection(&self) -> Result<u32> {
        guard("GetSelection", || Ok(self.this.snapshot.borrow().selection))
    }

    fn GetString(&self, uindex: u32) -> Result<BSTR> {
        guard("GetString", || {
            let snapshot = self.this.snapshot.borrow();
            let item = snapshot
                .items
                .get(uindex as usize)
                .ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;
            Ok(BSTR::from(item.as_str()))
        })
    }

    /// ページの区切りを教える。
    ///
    /// 置き場所を渡されない呼び方もある。そのときは**数だけ**を答える。
    /// アプリはまず数を尋ね、それから入れ物を用意して訊き直す。
    #[allow(
        clippy::not_unsafe_ptr_arg_deref,
        reason = "COM の口の形が決まっている"
    )]
    fn GetPageIndex(&self, pindex: *mut u32, usize_: u32, pupagecnt: *mut u32) -> Result<()> {
        guard("GetPageIndex", || {
            if pupagecnt.is_null() {
                return Err(E_INVALIDARG.into());
            }
            let starts = self.this.snapshot.borrow().page_starts();
            let count = u32::try_from(starts.len()).unwrap_or(0);

            if pindex.is_null() {
                // SAFETY: null でないことを確かめた書き込み先へ写す。
                unsafe { *pupagecnt = count };
                return Ok(());
            }

            let room = usize_.min(count) as usize;
            // SAFETY: 渡された入れ物の大きさを超えない範囲で書く。
            unsafe {
                std::ptr::copy_nonoverlapping(starts.as_ptr(), pindex, room);
                *pupagecnt = u32::try_from(room).unwrap_or(0);
            }
            Ok(())
        })
    }

    /// アプリからページを変えろと言われた。
    ///
    /// 受け付けない。**ページを送るのは打鍵の仕事**で、エンジンの状態と
    /// して動く。外から動かされると、一覧と見出しの対応が崩れる。
    fn SetPageIndex(&self, _pindex: *const u32, _upagecnt: u32) -> Result<()> {
        guard("SetPageIndex", || Err(E_INVALIDARG.into()))
    }

    fn GetCurrentPage(&self) -> Result<u32> {
        guard("GetCurrentPage", || {
            Ok(self.this.snapshot.borrow().current_page)
        })
    }
}

/// 申告した一覧と、その受付番号。
#[derive(Debug)]
pub struct Announced {
    element: ComObject<CandidateListElement>,
    id: u32,
}

impl Announced {
    /// 自前の窓を出してよいか。
    pub fn ours_to_draw(&self) -> bool {
        self.element.ours_to_draw()
    }

    /// 中身を入れ替え、変わったことを知らせる。
    pub fn update(
        &self,
        thread_manager: &ITfThreadMgr,
        snapshot: ListSnapshot,
        documents: Option<ITfDocumentMgr>,
    ) {
        self.element.set(snapshot, documents);
        let Ok(manager) = thread_manager.cast::<ITfUIElementMgr>() else {
            return;
        };
        // SAFETY: 受付番号は `begin` が返したもの。
        unsafe {
            let _ = manager.UpdateUIElement(self.id);
        }
    }

    /// 申告を取り下げる。
    pub fn end(self, thread_manager: &ITfThreadMgr) {
        let Ok(manager) = thread_manager.cast::<ITfUIElementMgr>() else {
            return;
        };
        // SAFETY: 受付番号は `begin` が返したもの。
        unsafe {
            let _ = manager.EndUIElement(self.id);
        }
    }
}

/// 一覧を持っていることを申告する。
///
/// 返るのは受付番号を抱えた持ち手。申告できなければ `None` で、その場合は
/// 自前の窓だけで進む。**普通のアプリではそれで困らない。**
pub fn begin(
    thread_manager: &ITfThreadMgr,
    snapshot: ListSnapshot,
    documents: Option<ITfDocumentMgr>,
) -> Option<Announced> {
    let manager = thread_manager.cast::<ITfUIElementMgr>().ok()?;
    let element = ComObject::new(CandidateListElement::new());
    element.set(snapshot, documents);

    let interface: ITfUIElement = element.to_interface();
    let mut ours = BOOL::from(true);
    let mut id = 0u32;
    // SAFETY: 差し出すのはこちらが生かし続ける実体。
    unsafe {
        manager
            .BeginUIElement(&interface, &mut ours, &mut id)
            .ok()?;
    }
    element.set_ours_to_draw(ours.as_bool());
    log::write(&format!(
        "候補一覧を申告した ({})",
        if ours.as_bool() {
            "自前で描く"
        } else {
            "アプリが描く"
        }
    ));
    Some(Announced { element, id })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(items: usize, page_size: u32) -> ListSnapshot {
        ListSnapshot {
            items: (0..items).map(|n| format!("候補{n}")).collect(),
            selection: 0,
            page_size,
            current_page: 0,
        }
    }

    #[test]
    fn pages_start_every_page_size_items() {
        assert_eq!(snapshot(15, 7).page_starts(), vec![0, 7, 14]);
    }

    #[test]
    fn a_short_list_is_one_page() {
        assert_eq!(snapshot(3, 7).page_starts(), vec![0]);
    }

    #[test]
    fn an_empty_list_has_no_pages() {
        assert!(snapshot(0, 7).page_starts().is_empty());
    }
}
