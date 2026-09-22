//! 文書への書き込み。
//!
//! TSF では、文書に触れてよいのは「編集セッション」の中だけである。
//! 触りたい側がセッションを実装して渡し、TSF が都合のよい時点で
//! 呼び返す。打鍵の処理中は同期的に呼び返してもらえる。
//!
//! 書き込みは「選択範囲を取り、その範囲の文字列を置き換え、カーソルを
//! 末尾へ移す」という手順で行う。`ITfInsertAtSelection` を使うほうが
//! 短く書けるが、そちらはアプリによってはそもそも実装されておらず、
//! 呼んだ先で落ちることがある。

use std::mem::ManuallyDrop;

use windows::Win32::UI::TextServices::{
    ITfContext, ITfEditSession, ITfEditSession_Impl, ITfRange, TF_AE_NONE, TF_ANCHOR_END,
    TF_DEFAULT_SELECTION, TF_ES_READWRITE, TF_ES_SYNC, TF_SELECTION, TF_SELECTIONSTYLE,
};
use windows::core::{Result, implement};

use crate::guard::guard;
use crate::log;

/// 確定した文字列を、いまのカーソル位置へ入れるセッション。
#[implement(ITfEditSession)]
pub struct InsertText {
    context: ITfContext,
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
            let range = selection_range(&self.this.context, ec)?;

            // SAFETY: 編集権 `ec` は TSF がこの呼び出しのために渡したもの。
            // 範囲は直前に受け取ったもので、文字列はこのセッションが持つ。
            unsafe {
                range.SetText(ec, 0, &self.this.text)?;
                log::write("文字列を書いた");

                // 書いた分だけカーソルを進める。これをしないと、次に
                // 書くときに同じ場所を上書きしてしまう。
                range.Collapse(ec, TF_ANCHOR_END)?;
                set_selection(&self.this.context, ec, &range)?;
            }
            log::write("カーソルを移した");
            Ok(())
        })
    }
}

/// いまの選択範囲を一つ取り出す。
fn selection_range(context: &ITfContext, ec: u32) -> Result<ITfRange> {
    let mut selections = [TF_SELECTION::default()];
    let mut fetched = 0u32;

    // SAFETY: 長さ付きの配列と、書き込み先の整数を渡している。
    unsafe {
        context.GetSelection(ec, TF_DEFAULT_SELECTION, &mut selections, &mut fetched)?;
    }
    log::write(&format!("選択範囲を取れた ({fetched} 件)"));

    // `TF_SELECTION` の範囲は手で落とす約束になっている。取り出して
    // 普通の持ち手に移し替えることで、以降は自動で解放される。
    let [selection] = selections;
    let range = ManuallyDrop::into_inner(selection.range);

    if fetched == 0 {
        return Err(windows::Win32::Foundation::E_FAIL.into());
    }
    range.ok_or_else(|| windows::Win32::Foundation::E_FAIL.into())
}

/// 選択範囲を、渡した範囲に合わせる。
///
/// # Safety
///
/// `ec` が有効な編集権であり、`range` がこの文脈のものであること。
unsafe fn set_selection(context: &ITfContext, ec: u32, range: &ITfRange) -> Result<()> {
    let selection = TF_SELECTION {
        // `TF_SELECTION` は範囲の所有権を持たない扱いなので、複製を
        // 包んで渡し、呼び出しの間だけ生かす。
        range: ManuallyDrop::new(Some(range.clone())),
        style: TF_SELECTIONSTYLE {
            ase: TF_AE_NONE,
            fInterimChar: false.into(),
        },
    };
    let selections = [selection];

    // SAFETY: 呼び出し側の約束による。
    let result = unsafe { context.SetSelection(ec, &selections) };

    // 包んだ複製をここで落とす。落とさないと参照が漏れる。
    let [selection] = selections;
    drop(ManuallyDrop::into_inner(selection.range));
    result
}

/// 文字列を文書へ入れる。
///
/// 打鍵の処理中なので同期の編集セッションを求める。TSF が断ることも
/// ありうるが、そのときは入力が届かないだけで、壊れはしない。
pub fn insert_text(context: &ITfContext, client_id: u32, text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }

    let session: ITfEditSession = InsertText {
        context: context.clone(),
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
