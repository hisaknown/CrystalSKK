//! テキスト入力プロセッサ本体。
//!
//! TSF はこのオブジェクトを**入力先アプリのプロセスの中で**生成する。
//! したがってここに置くものはすべて、他人のプロセスに同居してよいものに
//! 限られる。重い処理も、失敗しうる処理も、ここでは持たない (PRD §3)。
//!
//! いまは確定した文字列を文書へ入れるところまで。未確定の表示 (`▽` や
//! `▼`) はまだ出ない。

use std::cell::RefCell;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::TextServices::{
    ITfContext, ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfTextInputProcessor,
    ITfTextInputProcessor_Impl, ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl,
    ITfThreadMgr,
};
use windows::core::{BOOL, GUID, IUnknownImpl, Interface, Ref, Result, implement};

use crystalskk_core::Engine;
use crystalskk_core::dict::EmptyDict;

use crate::{edit, keys};

/// TSF から渡される、このスレッドでの立場。
#[derive(Debug)]
struct Activation {
    /// このプロセスでの CrystalSKK の識別子。編集セッションを頼むのに要る。
    client_id: u32,
    /// キーイベントの受け口を外すために持っておく。
    keystrokes: ITfKeystrokeMgr,
}

/// CrystalSKK の TIP。
#[implement(ITfTextInputProcessorEx, ITfTextInputProcessor, ITfKeyEventSink)]
pub struct TextService {
    /// 有効化されている間だけ中身が入る。
    ///
    /// TSF は単一スレッドアパートメントで呼ぶので、`RefCell` で足りる。
    activation: RefCell<Option<Activation>>,
    /// 変換の状態。
    ///
    /// 辞書はまだ繋いでいないので、変換はすべて辞書登録になる。
    engine: RefCell<Engine>,
}

impl Default for TextService {
    fn default() -> Self {
        Self::new()
    }
}

impl TextService {
    pub fn new() -> Self {
        Self {
            activation: RefCell::new(None),
            engine: RefCell::new(Engine::new(Box::new(EmptyDict))),
        }
    }

    /// 有効化されているか。
    pub fn is_active(&self) -> bool {
        self.activation.borrow().is_some()
    }

    /// 有効化されているときだけ、識別子を返す。
    fn client_id(&self) -> Option<u32> {
        self.activation.borrow().as_ref().map(|a| a.client_id)
    }

    fn deactivate(&self) -> Result<()> {
        let Some(activation) = self.activation.borrow_mut().take() else {
            return Ok(());
        };
        // 受け口を外す。外せなくても、保持していたものは落とす。
        // SAFETY: 有効化のときに受け取った識別子をそのまま返している。
        unsafe {
            let _ = activation
                .keystrokes
                .UnadviseKeyEventSink(activation.client_id);
        }
        self.engine.borrow_mut().reset();
        Ok(())
    }
}

impl std::fmt::Debug for TextService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextService")
            .field("active", &self.is_active())
            .finish_non_exhaustive()
    }
}

impl TextService_Impl {
    /// 有効化。キーイベントの受け口を登録する。
    fn activate(&self, thread_manager: Ref<ITfThreadMgr>, client_id: u32) -> Result<()> {
        // 二重の有効化に備えて、まず後始末をする。
        self.this.deactivate()?;

        let Some(thread_manager) = thread_manager.cloned() else {
            return Ok(());
        };
        let keystrokes: ITfKeystrokeMgr = thread_manager.cast()?;
        let sink: ITfKeyEventSink = self.to_interface();

        // SAFETY: 識別子と受け口はどちらもこの呼び出しのために用意したもの。
        unsafe {
            keystrokes.AdviseKeyEventSink(client_id, &sink, true)?;
        }

        *self.this.activation.borrow_mut() = Some(Activation {
            client_id,
            keystrokes,
        });
        Ok(())
    }

    /// 打鍵を処理し、確定した文字列を文書へ入れる。
    ///
    /// 戻り値は「この打鍵を食べたか」。食べなかった打鍵はアプリへ渡る。
    fn handle_key(&self, context: Ref<ITfContext>, wparam: WPARAM) -> BOOL {
        let Some(key) = keys::translate(wparam) else {
            return false.into();
        };
        let Some(client_id) = self.this.client_id() else {
            return false.into();
        };
        let Some(context) = context.as_ref() else {
            return false.into();
        };

        let response = self.this.engine.borrow_mut().press(key);
        if !response.commit.is_empty() {
            // 入れられなくても、エンジンの状態はもう進んでいる。ここで
            // 慌てても直せないので、食べたことだけは正しく伝える。
            let _ = edit::insert_text(context, client_id, &response.commit);
        }
        response.handled.into()
    }

    /// 打鍵を食べるかどうかだけを答える。状態は変えない。
    fn would_handle_key(&self, wparam: WPARAM) -> BOOL {
        let Some(key) = keys::translate(wparam) else {
            return false.into();
        };
        self.this.engine.borrow().would_handle(key).into()
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> Result<()> {
        self.activate(ptim, tid)
    }

    fn Deactivate(&self) -> Result<()> {
        self.this.deactivate()
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    /// `dwflags` は入力先の種類 (`TF_TMF_*`) を伝える。まだ使わない。
    fn ActivateEx(&self, ptim: Ref<ITfThreadMgr>, tid: u32, _dwflags: u32) -> Result<()> {
        self.activate(ptim, tid)
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    /// 入力先が変わった。入力の途中経過は続けようがないので捨てる。
    fn OnSetFocus(&self, _fforeground: BOOL) -> Result<()> {
        self.this.engine.borrow_mut().reset();
        Ok(())
    }

    fn OnTestKeyDown(
        &self,
        _pic: Ref<ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(self.would_handle_key(wparam))
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        Ok(self.handle_key(pic, wparam))
    }

    /// 離した打鍵は使わない。押した側だけで足りる。
    fn OnTestKeyUp(&self, _pic: Ref<ITfContext>, _wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        Ok(false.into())
    }

    fn OnKeyUp(&self, _pic: Ref<ITfContext>, _wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        Ok(false.into())
    }

    /// 横取りするキーはまだ登録していない。
    fn OnPreservedKey(&self, _pic: Ref<ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        Ok(false.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_inactive() {
        let service = TextService::new();
        assert!(!service.is_active());
    }

    #[test]
    fn deactivating_an_inactive_service_is_not_an_error() {
        let service = TextService::new();
        service.deactivate().expect("何もせず成功する");
        assert!(!service.is_active());
    }
}
