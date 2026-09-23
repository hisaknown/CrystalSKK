//! 言語バーの項目。
//!
//! 入力モードを示す小さな表示で、いわゆる「あ / A」のあれである。
//! CrystalSKK にとっては利用者への案内であると同時に、**TIP が生きて
//! いるかどうかを外から見る唯一の窓**でもある。これが出ないなら
//! 有効化そのものが起きていない。

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::UI::TextServices::{
    GUID_LBI_INPUTMODE, ITfLangBarItem, ITfLangBarItem_Impl, ITfLangBarItemButton,
    ITfLangBarItemButton_Impl, ITfLangBarItemMgr, ITfLangBarItemSink, ITfMenu, ITfSource,
    ITfSource_Impl, ITfThreadMgr, TF_LANGBARITEMINFO, TF_LBI_CLK_RIGHT, TF_LBI_STYLE_BTN_BUTTON,
    TF_LBI_STYLE_SHOWNINTRAY, TfLBIClick,
};
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows::core::{BSTR, GUID, IUnknown, Interface, Ref, Result, implement};

use crystalskk_core::InputMode;

use crate::guard::guard;
use crate::guids::CLSID_CRYSTALSKK;
use crate::icon;
use crate::log;
use crate::menu::{self, Command};

/// 品書きで選ばれたことを受け取る先。
type Handler = Rc<dyn Fn(Command)>;

/// 入力方式が切のときに出す文字。
///
/// どのモードでもないことを示す。モードの文字 (「あ」「A」…) を使うと、
/// 切れているのか英数なのかが見分けられなくなる。
const OFF_LABEL: &str = "－";

/// 並び順。小さいほど手前に出る。
const SORT_ORDER: u32 = 1;

/// 言語バーに出す入力モードの表示。
#[implement(ITfLangBarItemButton, ITfLangBarItem, ITfSource)]
pub struct ModeIndicator {
    /// いま出している表示。`None` は入力方式が切。
    shown: RefCell<Option<InputMode>>,
    /// 変化を知らせる相手。
    sinks: RefCell<Vec<(u32, ITfLangBarItemSink)>>,
    /// 次に配る受付番号。
    next_cookie: RefCell<u32>,
    /// 品書きで選ばれたことを渡す先。有効化されている間だけ入る。
    ///
    /// 渡す先は TIP 本体で、本体もこの表示を持っている。**無効化のときに
    /// 外さないと、互いに持ち合ったまま解放されない。**
    handler: RefCell<Option<Handler>>,
}

impl std::fmt::Debug for ModeIndicator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModeIndicator")
            .field("shown", &self.shown.borrow())
            .finish_non_exhaustive()
    }
}

impl Default for ModeIndicator {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeIndicator {
    pub fn new() -> Self {
        Self {
            shown: RefCell::new(None),
            sinks: RefCell::new(Vec::new()),
            next_cookie: RefCell::new(1),
            handler: RefCell::new(None),
        }
    }

    /// 品書きで選ばれたことを渡す先を決める。
    pub fn set_handler(&self, handler: impl Fn(Command) + 'static) {
        *self.handler.borrow_mut() = Some(Rc::new(handler));
    }

    /// 渡す先を外す。無効化のときに呼ぶ。
    pub fn clear_handler(&self) {
        self.handler.borrow_mut().take();
    }

    /// 選ばれたことを渡す。
    fn dispatch(&self, command: Command) {
        // 借用したまま呼ばない。呼んだ先で確かめの窓が出ている間に、
        // もう一度ここへ来ることがある。
        let handler = self.handler.borrow().clone();
        match handler {
            Some(handler) => handler(command),
            None => log::error(&format!("品書きの {command:?} を渡す先がありません")),
        }
    }

    /// 入力モードを表示する。入力方式が入のときに使う。
    pub fn set_mode(&self, mode: InputMode) {
        self.show(Some(mode));
    }

    /// 入力方式が切であることを表示する。
    ///
    /// **切もまた一つの状態であり、黙って前の表示を残してよいものではない。**
    /// 「あ」のまま切れていたら、打てないのに打てるように見える。
    pub fn set_off(&self) {
        self.show(None);
    }

    /// 表示を差し替え、言語バーに描き直させる。
    fn show(&self, shown: Option<InputMode>) {
        if *self.shown.borrow() == shown {
            return;
        }
        *self.shown.borrow_mut() = shown;

        // 知らせる相手を複製してから呼ぶ。借用したまま外へ出ると、
        // 呼んだ先から戻ってきたときに借用が重なってパニックになる。
        let sinks: Vec<ITfLangBarItemSink> = self
            .sinks
            .borrow()
            .iter()
            .map(|(_, sink)| sink.clone())
            .collect();

        // 変化を知らせないと、言語バーは古い表示のままになる。
        for sink in sinks {
            // SAFETY: 相手から預かった受け口をそのまま呼ぶ。
            unsafe {
                let _ = sink.OnUpdate(TF_LBI_STATUS | TF_LBI_TEXT | TF_LBI_ICON);
            }
        }
    }

    /// いま出す文字。
    ///
    /// 切のときは、どのモードでもない印を出す。
    fn label(&self) -> &'static str {
        match *self.shown.borrow() {
            Some(mode) => mode.label(),
            None => OFF_LABEL,
        }
    }
}

/// 変化の種類。`OnUpdate` に渡す。
const TF_LBI_STATUS: u32 = 0x0001;
const TF_LBI_ICON: u32 = 0x0002;
const TF_LBI_TEXT: u32 = 0x0004;

impl ITfLangBarItem_Impl for ModeIndicator_Impl {
    // 署名は COM が決めており、生ポインタを受け取る安全な関数にせざるを得ない。
    #[allow(
        clippy::not_unsafe_ptr_arg_deref,
        reason = "COM の呼び出し規約が引数の有効性を保証する"
    )]
    fn GetInfo(&self, pinfo: *mut TF_LANGBARITEMINFO) -> Result<()> {
        guard("GetInfo", || {
            if pinfo.is_null() {
                return Err(E_INVALIDARG.into());
            }

            let mut info = TF_LANGBARITEMINFO {
                clsidService: CLSID_CRYSTALSKK,
                // 独自の GUID ではなく、Windows が定める「入力モード」の
                // 項目として名乗る。トレイに出るのはこの項目だけであり、
                // 独自の GUID では言語バーに載っても人目に触れない。
                guidItem: GUID_LBI_INPUTMODE,
                // 押せる釦として、トレイにも出す。
                dwStyle: TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_SHOWNINTRAY,
                ulSort: SORT_ORDER,
                szDescription: [0; 32],
            };
            write_fixed(&mut info.szDescription, "CrystalSKK");

            // SAFETY: null でないことを確かめた書き込み先へ、埋めた値を写す。
            unsafe { *pinfo = info };
            Ok(())
        })
    }

    /// 隠す理由はないので、常に表示する。
    fn GetStatus(&self) -> Result<u32> {
        guard("GetStatus", || Ok(0))
    }

    fn Show(&self, _fshow: windows::core::BOOL) -> Result<()> {
        guard("Show", || Ok(()))
    }

    fn GetTooltipString(&self) -> Result<BSTR> {
        guard("GetTooltipString", || {
            Ok(BSTR::from("CrystalSKK の入力モード"))
        })
    }
}

impl ITfLangBarItemButton_Impl for ModeIndicator_Impl {
    /// 右クリックされたら品書きを出す。
    ///
    /// トレイの入力モード表示は、右クリックを `OnClick` で伝えてくる。
    /// 品書きを出すのはこちらの仕事である (CorvusSKK もそうしている)。
    /// 左クリックはまだ何もしない。
    #[allow(
        clippy::not_unsafe_ptr_arg_deref,
        reason = "COM の呼び出し規約が引数の有効性を保証する"
    )]
    fn OnClick(&self, click: TfLBIClick, pt: &POINT, prcarea: *const RECT) -> Result<()> {
        guard("OnClick", || {
            if click != TF_LBI_CLK_RIGHT {
                return Ok(());
            }
            // SAFETY: null でなければ、呼び出しの間は有効な領域である。
            let area = (!prcarea.is_null()).then(|| unsafe { *prcarea });
            if let Some(command) = menu::pop_up(*pt, area) {
                self.this.dispatch(command);
            }
            Ok(())
        })
    }

    /// 古い言語バーから品書きの中身を尋ねられた。
    fn InitMenu(&self, pmenu: Ref<ITfMenu>) -> Result<()> {
        guard("InitMenu", || match pmenu.as_ref() {
            Some(menu) => menu::fill(menu),
            None => Err(E_INVALIDARG.into()),
        })
    }

    /// 古い言語バーの品書きで選ばれた。
    fn OnMenuSelect(&self, wid: u32) -> Result<()> {
        guard("OnMenuSelect", || {
            if let Some(command) = Command::from_id(wid) {
                self.this.dispatch(command);
            }
            Ok(())
        })
    }

    /// トレイに出す絵。
    ///
    /// 文字を返しても描かれない。**絵がないと表示自体が出ない**ので、
    /// ここは必ず本物の `HICON` を返す必要がある。
    ///
    /// 返したアイコンは言語バー側が解放する。呼ばれるたびに作り直すのは
    /// そのため。拡大率が変わっても追随できる利点もある。
    fn GetIcon(&self) -> Result<HICON> {
        guard("GetIcon", || icon::render(self.this.label()))
    }

    fn GetText(&self) -> Result<BSTR> {
        guard("GetText", || Ok(BSTR::from(self.this.label())))
    }
}

impl ITfSource_Impl for ModeIndicator_Impl {
    #[allow(
        clippy::not_unsafe_ptr_arg_deref,
        reason = "COM の呼び出し規約が引数の有効性を保証する"
    )]
    fn AdviseSink(&self, riid: *const GUID, punk: Ref<IUnknown>) -> Result<u32> {
        guard("AdviseSink", || {
            // SAFETY: 呼び出し側が有効な GUID を渡すことは COM の約束。
            if riid.is_null() || unsafe { *riid } != ITfLangBarItemSink::IID {
                return Err(E_INVALIDARG.into());
            }
            let Some(sink) = punk
                .as_ref()
                .and_then(|u| u.cast::<ITfLangBarItemSink>().ok())
            else {
                return Err(E_INVALIDARG.into());
            };

            log::write("言語バーが変化の通知を求めてきた");
            let cookie = *self.this.next_cookie.borrow();
            *self.this.next_cookie.borrow_mut() = cookie.wrapping_add(1);
            self.this.sinks.borrow_mut().push((cookie, sink));
            Ok(cookie)
        })
    }

    fn UnadviseSink(&self, dwcookie: u32) -> Result<()> {
        guard("UnadviseSink", || {
            let mut sinks = self.this.sinks.borrow_mut();
            let before = sinks.len();
            sinks.retain(|(cookie, _)| *cookie != dwcookie);
            if sinks.len() == before {
                return Err(E_INVALIDARG.into());
            }
            Ok(())
        })
    }
}

/// 言語バーへ項目を出す。
pub fn add(thread_manager: &ITfThreadMgr, item: &ITfLangBarItem) -> Result<()> {
    let manager: ITfLangBarItemMgr = thread_manager.cast()?;
    // SAFETY: どちらもこの呼び出しのために用意した有効な参照。
    unsafe { manager.AddItem(item) }?;
    log::write("言語バーに項目を出した");
    Ok(())
}

/// 言語バーから項目を消す。
pub fn remove(thread_manager: &ITfThreadMgr, item: &ITfLangBarItem) {
    let Ok(manager) = thread_manager.cast::<ITfLangBarItemMgr>() else {
        return;
    };
    // SAFETY: 出したときと同じ項目を渡している。
    unsafe {
        let _ = manager.RemoveItem(item);
    }
}

/// 固定長の配列へ、終端を残して文字列を書く。
fn write_fixed(destination: &mut [u16; 32], text: &str) {
    let encoded: Vec<u16> = text.encode_utf16().collect();
    let length = encoded.len().min(destination.len() - 1);
    destination[..length].copy_from_slice(&encoded[..length]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_starts_off() {
        // 有効化されるまで、どのモードでもない。入力方式が入かどうかは
        // 区画を読むまで分からないので、勝手に「あ」を出さない。
        assert_eq!(ModeIndicator::new().label(), OFF_LABEL);
    }

    #[test]
    fn the_label_follows_the_mode() {
        let indicator = ModeIndicator::new();
        indicator.set_mode(InputMode::Hiragana);
        assert_eq!(indicator.label(), "あ");
        indicator.set_mode(InputMode::Ascii);
        assert_eq!(indicator.label(), "A");
    }

    #[test]
    fn off_is_not_any_mode() {
        let indicator = ModeIndicator::new();
        indicator.set_mode(InputMode::Ascii);
        indicator.set_off();
        // 切と半角英数を同じ顔にしてはいけない。どちらも「英字が入る」
        // ように見えて、片方は何も入らない。
        assert_ne!(indicator.label(), InputMode::Ascii.label());
        assert_eq!(indicator.label(), OFF_LABEL);
    }

    #[test]
    fn a_long_description_is_cut_and_still_terminated() {
        let mut buffer = [1u16; 32];
        write_fixed(&mut buffer, &"あ".repeat(100));
        assert_eq!(buffer[31], 1, "終端の分は書き換えない");
    }
}
