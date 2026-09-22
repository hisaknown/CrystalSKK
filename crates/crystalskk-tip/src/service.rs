//! テキスト入力プロセッサ本体。
//!
//! TSF はこのオブジェクトを**入力先アプリのプロセスの中で**生成する。
//! したがってここに置くものはすべて、他人のプロセスに同居してよいものに
//! 限られる。重い処理も、失敗しうる処理も、ここでは持たない (PRD §3)。
//!
//! いまは有効化と無効化を受け取るだけで、何も入力しない。

use std::cell::RefCell;

use windows::Win32::UI::TextServices::{
    ITfTextInputProcessor, ITfTextInputProcessor_Impl, ITfTextInputProcessorEx,
    ITfTextInputProcessorEx_Impl, ITfThreadMgr,
};
use windows::core::{Ref, Result, implement};

/// TSF から渡される、このスレッドでの立場。
///
/// 中身はまだ読まない。キー入力を受け取る段階で、ここから文書と編集の口を辿る。
#[derive(Debug)]
#[allow(dead_code, reason = "キー入力を受け取る段階で使う")]
struct Activation {
    /// スレッドマネージャ。ここから文書や編集の口を辿る。
    thread_manager: ITfThreadMgr,
    /// このプロセスでの CrystalSKK の識別子。
    client_id: u32,
}

/// CrystalSKK の TIP。
#[implement(ITfTextInputProcessorEx, ITfTextInputProcessor)]
pub struct TextService {
    /// 有効化されている間だけ中身が入る。
    ///
    /// TSF は単一スレッドアパートメントで呼ぶので、`RefCell` で足りる。
    activation: RefCell<Option<Activation>>,
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
        }
    }

    /// 有効化されているか。
    pub fn is_active(&self) -> bool {
        self.activation.borrow().is_some()
    }

    fn activate(&self, thread_manager: Ref<ITfThreadMgr>, client_id: u32) -> Result<()> {
        // 二重の有効化に備えて、まず後始末をする。
        self.deactivate()?;

        let Some(thread_manager) = thread_manager.cloned() else {
            return Ok(());
        };
        *self.activation.borrow_mut() = Some(Activation {
            thread_manager,
            client_id,
        });
        Ok(())
    }

    fn deactivate(&self) -> Result<()> {
        // 保持しているものを落とすだけ。TSF は無効化のあとも、このオブジェクト
        // 自体をしばらく生かしておくことがある。
        *self.activation.borrow_mut() = None;
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

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> Result<()> {
        self.this.activate(ptim, tid)
    }

    fn Deactivate(&self) -> Result<()> {
        self.this.deactivate()
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    /// `dwflags` は入力先の種類 (`TF_TMF_*`) を伝える。まだ使わない。
    fn ActivateEx(&self, ptim: Ref<ITfThreadMgr>, tid: u32, _dwflags: u32) -> Result<()> {
        self.this.activate(ptim, tid)
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
