//! 文書への書き込み。
//!
//! TSF では、文書に触れてよいのは「編集セッション」の中だけである。
//! 触りたい側がセッションを実装して渡し、TSF が都合のよい時点で
//! 呼び返す。打鍵の処理中は同期的に呼び返してもらえる。

use windows::Win32::UI::TextServices::{
    ITfContext, ITfEditSession, ITfEditSession_Impl, ITfInsertAtSelection, TF_ES_READWRITE,
    TF_ES_SYNC, TF_IAS_NOQUERY,
};
use windows::core::{Interface, Result, implement};

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
            let insert: ITfInsertAtSelection = self.this.context.cast()?;

            // SAFETY: 編集権 `ec` は TSF がこの呼び出しのために渡したもの。
            // 文字列はこのセッションが持っており、呼び出しより長く生きる。
            //
            // `TF_IAS_NOQUERY` を指定すると、入れた場所は返ってこない。
            // したがって返り値は見ない。失敗として扱ってはならない。
            unsafe {
                let _ = insert.InsertTextAtSelection(ec, TF_IAS_NOQUERY, &self.this.text);
            }
            log::write("文字列を入れた");
            Ok(())
        })
    }
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
