//! 文書への書き込み。
//!
//! TSF では、文書に触れてよいのは「編集セッション」の中だけである。
//! 触りたい側がセッションを実装して渡し、TSF が都合のよい時点で
//! 呼び返す。打鍵の処理中は同期的に呼び返してもらえる。
//!
//! # なぜ composition を通すのか
//!
//! 確定した文字列を入れるだけなら、選択範囲に直接書けばよさそうに見える。
//! だが実際にそれをすると、アプリによっては呼んだ先で落ちる。
//!
//! CorvusSKK をはじめ動いている実装は、**確定した文字列であっても必ず
//! composition を開いてその中に書き、書き終えてから閉じている**。TSF に
//! とって、入力方式が文書へ書く手段はそれしかないのだと考えるのが正しい。
//! ここでも同じ手順を踏む。
//!
//! 手順は次のとおり。
//!
//! 1. `TF_IAS_QUERYONLY` で、書き込む場所を表す範囲だけを受け取る
//! 2. その範囲に composition を開く
//! 3. composition の範囲へ文字列を書く
//! 4. 選択を composition の末尾へ動かす
//! 5. composition を閉じる

use std::mem::ManuallyDrop;

use windows::Win32::Foundation::E_FAIL;
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfContext, ITfContextComposition, ITfEditSession,
    ITfEditSession_Impl, ITfInsertAtSelection, ITfRange, TF_AE_NONE, TF_ANCHOR_END,
    TF_ANCHOR_START, TF_DEFAULT_SELECTION, TF_ES_READWRITE, TF_ES_SYNC, TF_IAS_QUERYONLY,
    TF_SELECTION, TF_SELECTIONSTYLE,
};
use windows::core::{Interface, Result, implement};

use crate::guard::guard;
use crate::log;

/// 確定した文字列を、いまのカーソル位置へ入れるセッション。
#[implement(ITfEditSession)]
pub struct InsertText {
    context: ITfContext,
    /// composition の終了を受け取る相手。
    sink: ITfCompositionSink,
    /// 入れる文字列。UTF-16 に直してある。
    text: Vec<u16>,
}

impl std::fmt::Debug for InsertText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InsertText")
            .field("length", &self.text.len())
            .finish_non_exhaustive()
    }
}

impl ITfEditSession_Impl for InsertText_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        guard("DoEditSession", || {
            log::write("編集セッションに入った");
            let context = &self.this.context;

            let composition = start_composition(context, ec, &self.this.sink)?;
            log::write("composition を開いた");

            // SAFETY: 編集権 `ec` は TSF がこの呼び出しのために渡したもので、
            // composition は直前にこの文脈へ開いたもの。
            let result = unsafe { write_into(context, ec, &composition, &self.this.text) };

            // 書けても書けなくても閉じる。開いたままにすると、文書が
            // 変換中の見た目のまま取り残される。
            // SAFETY: 同上。
            unsafe {
                let _ = composition.EndComposition(ec);
            }
            log::write("composition を閉じた");
            result
        })
    }
}

/// 書き込む場所に composition を開く。
fn start_composition(
    context: &ITfContext,
    ec: u32,
    sink: &ITfCompositionSink,
) -> Result<ITfComposition> {
    let insert: ITfInsertAtSelection = context.cast()?;

    // SAFETY: `TF_IAS_QUERYONLY` は文字を入れず、入れるべき場所だけを返す。
    // 文字列を渡さないことを長さ 0 で示す。
    let range = unsafe { insert.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])? };
    log::write("書き込む場所を受け取った");

    let compositions: ITfContextComposition = context.cast()?;
    // SAFETY: 範囲は直前に受け取ったもの、受け口はこちらが持つもの。
    unsafe { compositions.StartComposition(ec, &range, sink) }
}

/// composition の中へ文字列を書き、選択をその末尾へ動かす。
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
            log::write("選択範囲を取れなかった");
            return Err(E_FAIL.into());
        };

        // 選択が composition の外へ出ているなら書かない。アプリが
        // カーソルを動かした後に書き込むと、関係のない場所を壊す。
        if !covers(ec, &range, &selection_range)? {
            log::write("選択が composition の外にあるので書かない");
            return Err(E_FAIL.into());
        }

        range.SetText(ec, 0, text)?;
        log::write("文字列を書いた");

        // 書いた分の後ろへカーソルを送る。
        selection_range.ShiftEndToRange(ec, &range, TF_ANCHOR_END)?;
        selection_range.ShiftStartToRange(ec, &range, TF_ANCHOR_END)?;
        selection_range.Collapse(ec, TF_ANCHOR_START)?;

        set_selection(context, ec, &selection_range, style)?;
        log::write("カーソルを移した");
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

/// 文字列を文書へ入れる。
///
/// 打鍵の処理中なので同期の編集セッションを求める。TSF が断ることも
/// ありうるが、そのときは入力が届かないだけで、壊れはしない。
pub fn insert_text(
    context: &ITfContext,
    client_id: u32,
    sink: &ITfCompositionSink,
    text: &str,
) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }

    let session: ITfEditSession = InsertText {
        context: context.clone(),
        sink: sink.clone(),
        text: text.encode_utf16().collect(),
    }
    .into();

    log::write("編集セッションを頼む");
    // SAFETY: 文脈と識別子は TSF から受け取ったもの。
    let result =
        unsafe { context.RequestEditSession(client_id, &session, TF_ES_SYNC | TF_ES_READWRITE)? };
    log::write(&format!("編集セッションが終わった ({result:?})"));
    result.ok()
}
