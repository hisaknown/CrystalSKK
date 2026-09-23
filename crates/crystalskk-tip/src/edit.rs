//! 文書への書き込み。
//!
//! TSF では、文書に触れてよいのは「編集セッション」の中だけである。
//! 触りたい側がセッションを実装して渡し、TSF が都合のよい時点で
//! 呼び返す。打鍵の処理中は同期的に呼び返してもらえる。
//!
//! # composition を通す
//!
//! 文書へ書く手段は composition しかない (ADR-0008)。確定した文字列も
//! 未確定の文字列も、同じ composition の中へ書く。違うのは、書いたあとに
//! 閉じるかどうかだけである。
//!
//! - 確定した文字列 … 書いて、閉じる。閉じた時点で普通の文字になる
//! - 未確定の文字列 … 書いて、開いたままにする。次の打鍵で書き換える
//!
//! 開いたままの composition は打鍵をまたいで持ち越す必要があるため、
//! 呼ぶ側が預かる。このモジュールは受け取って、新しい状態を返す。
//!
//! # 見え方は区切りごとに貼る
//!
//! 未確定の文字列は一本ではない。見出し語、送り仮名、候補と役目が違い、
//! **それぞれに違う見え方を付けたい** (ADR-0017)。
//!
//! ここは「どう見せるか」を知らない。渡された番号を、渡された長さぶんの
//! 範囲に貼るだけである。**何をどう見せるかは [`crate::display`] が決める。**

use std::cell::RefCell;
use std::mem::ManuallyDrop;

use windows::Win32::Foundation::{E_FAIL, HWND, RECT};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};
use windows::Win32::UI::TextServices::{
    GUID_PROP_ATTRIBUTE, ITfComposition, ITfCompositionSink, ITfContext, ITfContextComposition,
    ITfEditSession, ITfEditSession_Impl, ITfInsertAtSelection, ITfRange, TF_AE_NONE, TF_ANCHOR_END,
    TF_ANCHOR_START, TF_DEFAULT_SELECTION, TF_ES_ASYNCDONTCARE, TF_ES_READWRITE, TF_ES_SYNC,
    TF_IAS_QUERYONLY, TF_SELECTION, TF_SELECTIONSTYLE,
};
use windows::core::{ComObject, Interface, Result, implement};

use crate::guard::guard;
use crate::log;

/// 文書の見え方を、確定と未確定の組に合わせるセッション。
#[implement(ITfEditSession)]
pub struct Update {
    context: ITfContext,
    /// composition の終了を受け取る相手。
    sink: ITfCompositionSink,
    /// 今回確定する文字列。
    commit: Vec<u16>,
    /// 今回見せる未確定の文字列。区切りごとに、貼る番号と中身の組。
    preedit: Vec<(u32, Vec<u16>)>,
    /// 入る前に開いていた composition。出るときに新しい状態を書き戻す。
    composition: RefCell<Option<ITfComposition>>,
    /// 未確定の文字列が画面上で占める矩形。
    ///
    /// 候補の窓をどこへ置くかは、これを見て決める。**編集権のある間しか
    /// 尋ねられない**ので、書き込みのついでにここで取っておく。
    extent: RefCell<Option<RECT>>,
    /// 入力先アプリの窓。候補の窓の親にする。
    owner: RefCell<Option<HWND>>,
}

impl std::fmt::Debug for Update {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Update")
            .field("commit", &self.commit.len())
            .field("preedit", &self.preedit.len())
            .finish_non_exhaustive()
    }
}

impl ITfEditSession_Impl for Update_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        guard("DoEditSession", || {
            let this = &self.this;
            let mut composition = this.composition.borrow_mut().take();

            // 確定した文字列は、書いてから閉じる。閉じた時点で、その文字は
            // 文書の一部になり、こちらの手を離れる。
            if !this.commit.is_empty() {
                let opened = open_if_needed(&this.context, ec, &this.sink, composition.take())?;
                // SAFETY: 編集権と composition はこの呼び出しのためのもの。
                unsafe {
                    write_into(&this.context, ec, &opened, &this.commit)?;
                    opened.EndComposition(ec)?;
                }
                log::trace("確定した文字列を書いて composition を閉じた");
            }

            // 未確定の文字列は、書いて開いたままにする。
            let preedit: Vec<u16> = this
                .preedit
                .iter()
                .flat_map(|(_, text)| text.iter().copied())
                .collect();
            if !preedit.is_empty() {
                let opened = open_if_needed(&this.context, ec, &this.sink, composition.take())?;
                // SAFETY: 同上。
                unsafe {
                    write_into(&this.context, ec, &opened, &preedit)?;
                    // 書いたあとで貼る。**書く前に貼っても、書き換えで
                    // 消える。**
                    let _ = mark_segments(&this.context, ec, &opened, &this.preedit);
                }
                log::trace("未確定の文字列を書いた");
                composition = Some(opened);
            } else if let Some(opened) = composition.take() {
                // 見せるものがなくなったので、跡を消して閉じる。
                // SAFETY: 同上。
                unsafe {
                    let _ = write_into(&this.context, ec, &opened, &[]);
                    let _ = opened.EndComposition(ec);
                }
                log::trace("未確定がなくなったので composition を閉じた");
            }

            // 窓の置き場所を、composition が開いているうちに尋ねる。
            *this.extent.borrow_mut() = composition
                .as_ref()
                .and_then(|opened| text_extent(&this.context, ec, opened));
            *this.owner.borrow_mut() = owner_window(&this.context);

            *this.composition.borrow_mut() = composition;
            Ok(())
        })
    }
}

/// 区切りごとに見え方の番号を貼る。
///
/// 貼れなくても入力は続く。下線の見え方が既定に戻るだけである。
///
/// # Safety
///
/// `ec` が書き込み可能な編集権であり、`composition` がこの文脈のもので
/// あること。
unsafe fn mark_segments(
    context: &ITfContext,
    ec: u32,
    composition: &ITfComposition,
    segments: &[(u32, Vec<u16>)],
) -> Result<()> {
    // SAFETY: 呼び出し側の約束による。
    unsafe {
        let property = context.GetProperty(&GUID_PROP_ATTRIBUTE)?;
        let whole = composition.GetRange()?;

        // 先頭から順に、区切りの長さぶんだけ切り出して貼る。
        let cursor = whole.Clone()?;
        cursor.Collapse(ec, TF_ANCHOR_START)?;
        for (atom, text) in segments {
            let length = i32::try_from(text.len()).unwrap_or(0);
            if length == 0 {
                continue;
            }

            let piece = cursor.Clone()?;
            let mut moved = 0;
            piece.ShiftEnd(ec, length, &mut moved, std::ptr::null())?;
            if moved == 0 {
                break;
            }
            property.SetValue(ec, &piece, &integer(*atom as i32))?;

            let mut advanced = 0;
            cursor.ShiftStart(ec, moved, &mut advanced, std::ptr::null())?;
            if advanced == 0 {
                break;
            }
        }
        Ok(())
    }
}

/// 整数を入れた値を作る。
///
/// `VARIANT` は共用体の入れ子なので、組み立ててから包む。
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
            Anonymous: ManuallyDrop::new(inner),
        },
    }
}

/// 未確定の文字列が画面のどこにあるかを尋ねる。
///
/// answer は画面座標。取れないことはあり、そのときは窓を出す位置が
/// 決まらないだけなので、入力そのものは続ける。
fn text_extent(context: &ITfContext, ec: u32, composition: &ITfComposition) -> Option<RECT> {
    // SAFETY: 編集権はこの呼び出しのためのもので、範囲は composition のもの。
    unsafe {
        let view = context.GetActiveView().ok()?;
        let range = composition.GetRange().ok()?;
        let mut rect = RECT::default();
        let mut clipped = windows::core::BOOL::default();
        view.GetTextExt(ec, &range, &mut rect, &mut clipped).ok()?;
        // 潰れた矩形は当てにならない。アプリがまだ描いていないことがある。
        (rect.right > rect.left || rect.bottom > rect.top).then_some(rect)
    }
}

/// 入力先アプリの窓を尋ねる。
///
/// 候補の窓はこれを親にして作る。**親のないポップアップは、アプリの
/// 描画面の下に潜って見えないことがある。** ストアアプリがそうだった。
///
/// 文脈が窓を持たないこともある。そのときは焦点のある窓で代える。
fn owner_window(context: &ITfContext) -> Option<HWND> {
    // SAFETY: どちらも問い合わせるだけ。
    unsafe {
        let from_view = context
            .GetActiveView()
            .ok()
            .and_then(|view| view.GetWnd().ok())
            .filter(|hwnd| !hwnd.is_invalid());
        from_view.or_else(|| {
            let focused = windows::Win32::UI::Input::KeyboardAndMouse::GetFocus();
            (!focused.is_invalid()).then_some(focused)
        })
    }
}

/// composition がなければ開く。あればそのまま使う。
fn open_if_needed(
    context: &ITfContext,
    ec: u32,
    sink: &ITfCompositionSink,
    existing: Option<ITfComposition>,
) -> Result<ITfComposition> {
    if let Some(composition) = existing {
        return Ok(composition);
    }

    let insert: ITfInsertAtSelection = context.cast()?;
    // SAFETY: `TF_IAS_QUERYONLY` は文字を入れず、入れるべき場所だけを返す。
    // 文字列を渡さないことを長さ 0 で示す。
    let range = unsafe { insert.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])? };

    let compositions: ITfContextComposition = context.cast()?;
    // SAFETY: 範囲は直前に受け取ったもの、受け口はこちらが持つもの。
    let composition = unsafe { compositions.StartComposition(ec, &range, sink) }?;
    log::trace("composition を開いた");
    Ok(composition)
}

/// composition の中身を入れ替え、選択をその末尾へ動かす。
///
/// # Safety
///
/// `ec` が書き込み可能な編集権であり、`composition` がこの文脈のもので
/// あること。
unsafe fn write_into(
    context: &ITfContext,
    ec: u32,
    composition: &ITfComposition,
    text: &[u16],
) -> Result<()> {
    // SAFETY: 呼び出し側の約束による。
    unsafe {
        let range = composition.GetRange()?;

        let mut selections = [TF_SELECTION::default()];
        let mut fetched = 0u32;
        context.GetSelection(ec, TF_DEFAULT_SELECTION, &mut selections, &mut fetched)?;

        // `TF_SELECTION` の範囲は手で落とす約束になっている。普通の持ち手へ
        // 移し替えて、以降は自動で解放されるようにする。
        let [selection] = selections;
        let style = selection.style;
        let selection_range = ManuallyDrop::into_inner(selection.range);
        let Some(selection_range) = selection_range.filter(|_| fetched == 1) else {
            log::error("選択範囲を取れなかった");
            return Err(E_FAIL.into());
        };

        // 選択が composition の外へ出ているなら書かない。アプリが
        // カーソルを動かした後に書き込むと、関係のない場所を壊す。
        if !covers(ec, &range, &selection_range)? {
            log::trace("選択が composition の外にあるので書かない");
            return Err(E_FAIL.into());
        }

        range.SetText(ec, 0, text)?;

        // 書いた分の後ろへカーソルを送る。
        selection_range.ShiftEndToRange(ec, &range, TF_ANCHOR_END)?;
        selection_range.ShiftStartToRange(ec, &range, TF_ANCHOR_END)?;
        selection_range.Collapse(ec, TF_ANCHOR_START)?;

        set_selection(context, ec, &selection_range, style)?;
    }
    Ok(())
}

/// `outer` が `inner` を覆っているか。
///
/// # Safety
///
/// `ec` が有効な編集権であり、二つの範囲が同じ文脈のものであること。
unsafe fn covers(ec: u32, outer: &ITfRange, inner: &ITfRange) -> Result<bool> {
    // SAFETY: 呼び出し側の約束による。
    unsafe {
        if outer.CompareStart(ec, inner, TF_ANCHOR_START)? > 0 {
            return Ok(false);
        }
        Ok(outer.CompareEnd(ec, inner, TF_ANCHOR_END)? >= 0)
    }
}

/// 選択範囲を、渡した範囲に合わせる。
///
/// # Safety
///
/// `ec` が有効な編集権であり、`range` がこの文脈のものであること。
unsafe fn set_selection(
    context: &ITfContext,
    ec: u32,
    range: &ITfRange,
    style: TF_SELECTIONSTYLE,
) -> Result<()> {
    let selections = [TF_SELECTION {
        // 範囲の所有権は渡さない約束なので、複製を包んで渡し、
        // 呼び出しが終わったらここで落とす。
        range: ManuallyDrop::new(Some(range.clone())),
        style: TF_SELECTIONSTYLE {
            ase: TF_AE_NONE,
            ..style
        },
    }];

    // SAFETY: 呼び出し側の約束による。
    let result = unsafe { context.SetSelection(ec, &selections) };

    let [selection] = selections;
    drop(ManuallyDrop::into_inner(selection.range));
    result
}

/// 一度の書き込みで分かったこと。
#[derive(Debug, Default)]
pub struct Applied {
    /// 次に持ち越す composition。`None` なら開いていない。
    pub composition: Option<ITfComposition>,
    /// 未確定の文字列が画面上で占める矩形。候補の窓を置く目印。
    pub extent: Option<RECT>,
    /// 入力先アプリの窓。候補の窓の親にする。
    pub owner: Option<HWND>,
}

/// 文書の見え方を、確定と未確定の組に合わせる。
///
/// `composition` には前回から持ち越した composition を渡す。
///
/// 未確定の文字列の画面上の位置も、**同じセッションの中で**尋ねて返す。
/// 位置を訊くには編集権が要るので、別に取り直すと一打鍵あたり二度
/// セッションを頼むことになる。
///
/// 打鍵の処理中なので同期の編集セッションを求める。TSF が断ることも
/// ありうるが、そのときは入力が届かないだけで、壊れはしない。
pub fn update(
    context: &ITfContext,
    client_id: u32,
    sink: &ITfCompositionSink,
    commit: &str,
    preedit: &[(u32, String)],
    composition: Option<ITfComposition>,
) -> Result<Applied> {
    let empty = preedit.iter().all(|(_, text)| text.is_empty());
    if commit.is_empty() && empty && composition.is_none() {
        return Ok(Applied::default());
    }

    let session = ComObject::new(Update {
        context: context.clone(),
        sink: sink.clone(),
        commit: commit.encode_utf16().collect(),
        preedit: preedit
            .iter()
            .map(|(atom, text)| (*atom, text.encode_utf16().collect()))
            .collect(),
        composition: RefCell::new(composition),
        extent: RefCell::new(None),
        owner: RefCell::new(None),
    });
    let requested: ITfEditSession = session.to_interface();

    // SAFETY: 文脈と識別子は TSF から受け取ったもの。
    let result =
        unsafe { context.RequestEditSession(client_id, &requested, TF_ES_SYNC | TF_ES_READWRITE)? };
    result.ok()?;

    Ok(Applied {
        composition: session.composition.borrow_mut().take(),
        extent: *session.extent.borrow(),
        owner: *session.owner.borrow(),
    })
}

/// 開いたままの composition を、文書から取り除く。
///
/// 入力先が変わったときなど、続きを入力しようがない場面で呼ぶ。打鍵の
/// 途中ではないので、同期でなくてよい。
pub fn terminate(context: &ITfContext, client_id: u32, composition: ITfComposition) {
    let session: ITfEditSession = Terminate {
        composition: RefCell::new(Some(composition)),
    }
    .into();

    // SAFETY: 文脈と識別子は TSF から受け取ったもの。
    unsafe {
        let _ =
            context.RequestEditSession(client_id, &session, TF_ES_ASYNCDONTCARE | TF_ES_READWRITE);
    }
}

/// 開いたままの composition を閉じるだけのセッション。
#[implement(ITfEditSession)]
struct Terminate {
    composition: RefCell<Option<ITfComposition>>,
}

impl std::fmt::Debug for Terminate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminate").finish_non_exhaustive()
    }
}

impl ITfEditSession_Impl for Terminate_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        guard("DoEditSession(Terminate)", || {
            let Some(composition) = self.this.composition.borrow_mut().take() else {
                return Ok(());
            };
            // SAFETY: 編集権はこの呼び出しのために渡されたもの。
            unsafe {
                if let Ok(range) = composition.GetRange() {
                    let _ = range.SetText(ec, 0, &[]);
                }
                let _ = composition.EndComposition(ec);
            }
            log::trace("composition を片付けた");
            Ok(())
        })
    }
}
