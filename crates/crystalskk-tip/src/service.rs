//! テキスト入力プロセッサ本体。
//!
//! TSF はこのオブジェクトを**入力先アプリのプロセスの中で**生成する。
//! したがってここに置くものはすべて、他人のプロセスに同居してよいものに
//! 限られる。重い処理も、失敗しうる処理も、ここでは持たない (PRD §3)。
//!
//! COM から呼ばれる入口はすべて [`crate::guard::guard`] を通す。パニックを
//! そのまま外へ出すと、入力先アプリごと落ちる。
//!
//! 確定した文字列も未確定の表示 (`▽` `▼`) も、composition を通して
//! 文書へ書く (ADR-0008)。開いたままの composition はここで預かる。

use std::cell::RefCell;

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::TextServices::GUID_COMPARTMENT_KEYBOARD_OPENCLOSE;
use windows::Win32::UI::TextServices::{
    IEnumTfDisplayAttributeInfo, ITfCompartmentEventSink, ITfCompartmentEventSink_Impl,
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext,
    ITfDisplayAttributeInfo, ITfDisplayAttributeProvider, ITfDisplayAttributeProvider_Impl,
    ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfLangBarItem, ITfTextInputProcessor,
    ITfTextInputProcessor_Impl, ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl,
    ITfThreadMgr,
};
use windows::core::{
    BOOL, ComObject, GUID, IUnknown, IUnknownImpl, Interface, Ref, Result, implement,
};

use crystalskk_core::engine::Event;
use crystalskk_core::{Engine, InputMode};

use crate::candwin::{CandidateWindow, Content, Page, Registration};
use crate::dict::UNREACHABLE_NOTICE;
use crate::dict::{Learning, SharedSource};
use crate::guard::guard;
use crate::guids::{GUID_PRESERVED_KEY_OFF, GUID_PRESERVED_KEY_ON};
use crate::langbar::ModeIndicator;
use crate::uielement::{self, Announced, ListSnapshot};
use crate::{compartment, dict, edit, keys, langbar, log, preserved};

/// 入力方式が入にされた直後の入力モード。
///
/// 半角英数から始める。入にした時点では、利用者はまだ日本語を打つと
/// 決めていない。**入にしただけで打鍵の意味が変わる**と、英字を打つ
/// つもりだった人がかなを掴まされる。
///
/// かなへは `Ctrl+J` で自分から入る。SKK ではそれが普通の入り方で、
/// 一手増えることにはならない。
const DEFAULT_MODE: InputMode = InputMode::Ascii;

/// TSF から渡される、このスレッドでの立場。
#[derive(Debug)]
struct Activation {
    /// このプロセスでの CrystalSKK の識別子。編集セッションを頼むのに要る。
    client_id: u32,
    /// キーイベントの受け口を外すために持っておく。
    keystrokes: ITfKeystrokeMgr,
    /// 言語バーから項目を外すために持っておく。
    thread_manager: ITfThreadMgr,
    /// 言語バーに出している入力モードの表示。
    indicator: ITfLangBarItem,
    /// 表示を書き換えるための、実体への持ち手。
    indicator_object: ComObject<ModeIndicator>,
    /// 入切の区画を見張るための受付番号。外すときに要る。
    open_close_cookie: Option<u32>,
}

/// CrystalSKK の TIP。
#[implement(
    ITfTextInputProcessorEx,
    ITfTextInputProcessor,
    ITfKeyEventSink,
    ITfCompositionSink,
    ITfCompartmentEventSink,
    ITfDisplayAttributeProvider
)]
pub struct TextService {
    /// 有効化されている間だけ中身が入る。
    ///
    /// TSF は単一スレッドアパートメントで呼ぶので、`RefCell` で足りる。
    activation: RefCell<Option<Activation>>,
    /// 変換の状態。
    engine: RefCell<Engine>,
    /// 学習の書き込み先。
    learning: Learning,
    /// 辞書サーバとの繋がり。引けたかどうかを見るのに持つ。
    source: SharedSource,
    /// 未確定の文字列を見せている composition と、その文書。
    ///
    /// 打鍵をまたいで持ち越す。開いていなければ `None`。
    composition: RefCell<Option<(ITfContext, ITfComposition)>>,
    /// 候補の一覧と辞書登録を出す小窓。出す段階になるまで作らない。
    candidates: CandidateWindow,
    /// 最後に分かった、入力先アプリの窓。候補の窓の親にする。
    owner: RefCell<Option<windows::Win32::Foundation::HWND>>,
    /// 最後に分かった、未確定の文字列の画面上の位置。
    ///
    /// 辞書登録中は文書に何も書かないので、位置を尋ねる相手がいない。
    /// **直前まで書いていた場所のそばに出すのが、いちばん近い当て推量**
    /// になる。
    anchor: RefCell<Option<windows::Win32::Foundation::RECT>>,
    /// 見え方に振られた番号。有効化のときに一度取る。
    ///
    /// 取れなければ既定の下線のままになるだけで、入力は続く。
    atoms: RefCell<Option<crate::display::Atoms>>,
    /// システムへ申告している候補一覧。出していない間は `None`。
    ///
    /// **アプリが自分で描くと言うことがある。** そのときは自前の窓を
    /// 出さず、中身だけを渡す。
    announced: RefCell<Option<Announced>>,
}

impl Default for TextService {
    fn default() -> Self {
        Self::new()
    }
}

impl TextService {
    pub fn new() -> Self {
        let (engine, source, learning) = dict::build();
        Self {
            activation: RefCell::new(None),
            engine: RefCell::new(engine),
            learning,
            source,
            composition: RefCell::new(None),
            candidates: CandidateWindow::new(),
            owner: RefCell::new(None),
            anchor: RefCell::new(None),
            atoms: RefCell::new(None),
            announced: RefCell::new(None),
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

    /// 有効化されているときだけ、スレッドの持ち手を複製して返す。
    ///
    /// 借用を COM の呼び出しより長く持たないよう、必ず複製で渡す。
    fn thread_manager(&self) -> Option<ITfThreadMgr> {
        self.activation
            .borrow()
            .as_ref()
            .map(|a| a.thread_manager.clone())
    }

    /// 有効化されているときだけ、打鍵の管理者と識別子を複製して返す。
    fn keystroke_manager(&self) -> Option<(ITfKeystrokeMgr, u32)> {
        self.activation
            .borrow()
            .as_ref()
            .map(|a| (a.keystrokes.clone(), a.client_id))
    }

    /// いま打鍵を受け取ってよいか。
    ///
    /// 入力方式が切なら受け取らない。入力先が文字を断っていても受け取らない。
    /// **どちらも向こうが決めることで、こちらの入力モードとは関係がない。**
    fn accepts_keys(&self) -> bool {
        let Some(thread_manager) = self.thread_manager() else {
            return false;
        };
        compartment::is_open(&thread_manager) && compartment::accepts_input(&thread_manager)
    }

    /// 入切の区画を読み直し、こちらの状態を合わせる。
    ///
    /// 入なら入力を受けられる状態にし、切なら途中経過を捨てる。
    /// **区画が本当で、こちらが従う** (ADR-0012)。
    fn sync_with_open_state(&self) {
        let Some(thread_manager) = self.thread_manager() else {
            return;
        };
        let open = compartment::is_open(&thread_manager);
        log::write(&format!("入切を読んだ: {}", if open { "入" } else { "切" }));

        // 入切のキーを、いまの状態に合わせて登録し直す。入切を兼ねる
        // キーの意味は登録の順で決まるので、状態が変わるたびにやり直す。
        // 入にする手立てが無ければ、切られたまま二度と戻らない。
        if let Some((keystrokes, client_id)) = self.keystroke_manager() {
            preserved::unregister(&keystrokes, client_id);
            preserved::register(&keystrokes, client_id, open);
        }

        if open {
            // 入にされた直後は半角英数から始める。**入にしただけで打鍵の
            // 意味が変わらない**ほうがよい。かなへは `Ctrl+J` で自分から
            // 入る、という SKK の作法にも合う。
            self.engine.borrow_mut().restart_in(DEFAULT_MODE);
            self.show_mode();
        } else {
            self.drop_composition();
            self.engine.borrow_mut().reset();
            self.show_off();
        }
    }

    /// 学習と辞書登録をユーザー辞書へ反映する。
    ///
    /// 書くのは辞書サーバである。**こちらはファイルに触れない** (ADR-0016)。
    /// 書き手が一つに絞られているので、学習が競り合って壊れることがない。
    fn apply_events(&self, events: &[Event]) {
        for event in events {
            match event {
                Event::Learn { query, word } => {
                    self.learning.learn(query.clone(), word.clone());
                }
                Event::Register { query, word } => {
                    self.learning.register(query.clone(), word.clone());
                }
            }
        }
    }

    /// いまのモードを外へ映す。
    ///
    /// 言語バーの項目に知らせるだけでは足りない。Windows の表示は
    /// 区画に書いた値を見ているので、そちらにも書く (ADR-0010)。
    fn show_mode(&self) {
        let mode = self.engine.borrow().mode();

        // 借用を COM の呼び出しより長く持たない。呼び出しの先から
        // 戻ってこられると、借用が重なってパニックになる。
        let published = self.activation.borrow().as_ref().map(|a| {
            (
                a.thread_manager.clone(),
                a.client_id,
                a.indicator_object.clone(),
            )
        });

        let Some((thread_manager, client_id, indicator)) = published else {
            return;
        };
        indicator.set_mode(mode);
        compartment::publish_mode(&thread_manager, client_id, mode);
    }

    /// 小窓を、いまの状態に合わせる。
    ///
    /// 出すかどうかはエンジンが決めている。ここは「出せと言われたら出す」
    /// だけで、**何回目の変換かといった判断をこちらへ持ち込まない**。
    ///
    /// `extent` は未確定の文字列の画面上の位置。取れたら覚えておき、
    /// 取れなかったときは最後に分かった場所を使う。辞書登録中は文書に
    /// 何も書かないので、尋ねても返ってこない。
    fn show_window(
        &self,
        extent: Option<windows::Win32::Foundation::RECT>,
        owner: Option<windows::Win32::Foundation::HWND>,
    ) {
        if extent.is_some() {
            *self.anchor.borrow_mut() = extent;
        }
        if owner.is_some() {
            *self.owner.borrow_mut() = owner;
        }

        let Some(content) = self.window_content() else {
            self.withdraw_list();
            self.candidates.hide();
            return;
        };

        // 候補の一覧はシステムへも差し出す。アプリが自分で描くと言えば、
        // 自前の窓は出さない。辞書登録には差し出す口が無いので、
        // 自前の窓だけで出す。
        let ours_to_draw = match &content {
            Content::Page(_) => self.announce_list(),
            Content::Registration(_) | Content::Notice(_) => {
                self.withdraw_list();
                true
            }
        };
        if !ours_to_draw {
            self.candidates.hide();
            return;
        }

        let Some(anchor) = *self.anchor.borrow() else {
            log::trace("出す場所が分からないので小窓を出さない");
            self.candidates.hide();
            return;
        };
        self.candidates.show(&content, anchor, *self.owner.borrow());
    }

    /// 候補一覧をシステムへ差し出す。返るのは自前の窓を出してよいか。
    ///
    /// すでに差し出しているなら中身を入れ替えるだけにする。**打鍵のたびに
    /// 申告し直すと、アプリから見て一覧が消えては現れることになる。**
    fn announce_list(&self) -> bool {
        let Some(thread_manager) = self.thread_manager() else {
            return true;
        };
        let snapshot = self.list_snapshot();
        // SAFETY: 焦点を尋ねるだけ。取れなくても差し支えない。
        let documents = unsafe { thread_manager.GetFocus() }.ok();

        let mut announced = self.announced.borrow_mut();
        match announced.as_ref() {
            Some(existing) => {
                existing.update(&thread_manager, snapshot, documents);
                existing.ours_to_draw()
            }
            None => match uielement::begin(&thread_manager, snapshot, documents) {
                Some(fresh) => {
                    let ours = fresh.ours_to_draw();
                    *announced = Some(fresh);
                    ours
                }
                // 差し出せなくても自前の窓では出せる。普通のアプリでは
                // それで困らない。
                None => true,
            },
        }
    }

    /// 差し出していた一覧を取り下げる。
    fn withdraw_list(&self) {
        let Some(announced) = self.announced.borrow_mut().take() else {
            return;
        };
        let Some(thread_manager) = self.thread_manager() else {
            return;
        };
        announced.end(&thread_manager);
    }

    /// システムへ渡す一覧の中身。
    ///
    /// 渡すのは**一覧に載る候補だけ**である。一つずつ見せていた分まで
    /// 入れると、ページの区切りが合わなくなる。
    fn list_snapshot(&self) -> ListSnapshot {
        let engine = self.engine.borrow();
        let Some(view) = engine.candidates() else {
            return ListSnapshot::default();
        };
        let okuri = view.okuri.as_deref().unwrap_or("");
        let listed: Vec<String> = view
            .listed()
            .iter()
            .map(|candidate| format!("{}{okuri}", candidate.word))
            .collect();
        ListSnapshot {
            items: listed,
            selection: u32::try_from(view.page_number() * crystalskk_core::engine::PAGE_SIZE)
                .unwrap_or(0),
            page_size: u32::try_from(crystalskk_core::engine::PAGE_SIZE).unwrap_or(1),
            current_page: u32::try_from(view.page_number()).unwrap_or(0),
        }
    }

    /// いま小窓に出すもの。出すものが無ければ `None`。
    ///
    /// 辞書登録を先に見る。登録中は候補の選択も入れ子で起きうるが、
    /// **利用者にとって手前にあるのは登録のほう**である。
    fn window_content(&self) -> Option<Content> {
        let engine = self.engine.borrow();

        // 辞書サーバに届かなかったことを言う。**「その語は辞書に無い」と
        // 見分けがつかないまま進ませない。**
        //
        // 見出し語を入力している間だけ出す。確定すれば消える — 知らせを
        // 閉じる手立てを別に覚えてもらう必要がない。
        if self.source.was_unreachable() && !engine.preedit().is_empty() {
            return Some(Content::Notice(UNREACHABLE_NOTICE.to_owned()));
        }

        if let Some(registration) = engine.registration() {
            let key = match &registration.okuri {
                Some(okuri) => format!("{}{okuri}", registration.key),
                None => registration.key.clone(),
            };
            // 溜まった語と、いま打ちかけの文字列を繋いで見せる。
            // 打ちかけの分を落とすと、打った字が消えたように見える。
            let text = format!("{}{}", registration.buffer, engine.preedit().display());
            return Some(Content::Registration(Registration {
                key,
                text,
                depth: registration.depth,
            }));
        }

        let view = engine.candidates().filter(|view| view.listing)?;
        let okuri = view.okuri.as_deref().unwrap_or("");
        Some(Content::Page(Page {
            entries: view
                .page()
                .into_iter()
                .map(|(label, candidate)| (label, format!("{}{okuri}", candidate.word)))
                .collect(),
            number: view.page_number() + 1,
            count: view.page_count(),
        }))
    }

    /// 入力方式が切であることを表示に出す。
    ///
    /// 切ったときに何もしないと、**前のモードの顔のまま残る**。打てないのに
    /// 打てるように見えるので、状態の表示としては最悪の部類になる。
    fn show_off(&self) {
        let indicator = self
            .activation
            .borrow()
            .as_ref()
            .map(|a| a.indicator_object.clone());
        if let Some(indicator) = indicator {
            indicator.set_off();
        }
    }

    /// 開いたままの composition を片付ける。
    ///
    /// 候補の窓も一緒に畳む。**未確定の文字列が消えたのに一覧だけ残ると、
    /// どこに対する候補なのか分からなくなる。**
    fn drop_composition(&self) {
        self.withdraw_list();
        self.candidates.hide();
        self.drop_composition_only();
    }

    /// composition だけを片付ける。
    fn drop_composition_only(&self) {
        let Some((context, composition)) = self.composition.borrow_mut().take() else {
            return;
        };
        let Some(client_id) = self.client_id() else {
            return;
        };
        edit::terminate(&context, client_id, composition);
    }

    fn deactivate(&self) -> Result<()> {
        self.withdraw_list();
        self.drop_composition();
        self.candidates.close();
        self.learning.save();
        let Some(activation) = self.activation.borrow_mut().take() else {
            return Ok(());
        };
        log::write("無効化された");
        if let Some(cookie) = activation.open_close_cookie {
            compartment::unadvise_open_close(&activation.thread_manager, cookie);
        }
        preserved::unregister(&activation.keystrokes, activation.client_id);
        langbar::remove(&activation.thread_manager, &activation.indicator);
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
        log::write(&format!("有効化された (識別子 {client_id})"));

        let keystrokes: ITfKeystrokeMgr = thread_manager.cast()?;
        let sink: ITfKeyEventSink = self.to_interface();

        // SAFETY: 識別子と受け口はどちらもこの呼び出しのために用意したもの。
        unsafe {
            keystrokes.AdviseKeyEventSink(client_id, &sink, true)?;
        }
        log::write("キーイベントの受け口を登録した");

        // 言語バーの項目は、出せなくても入力そのものは続けられる。
        let indicator_object = ComObject::new(ModeIndicator::new());
        let indicator: ITfLangBarItem = indicator_object.to_interface();
        if let Err(e) = langbar::add(&thread_manager, &indicator) {
            log::error(&format!("言語バーに項目を出せなかった: {}", e.message()));
        }

        // 入切の変化を知らせてもらう。**これを聞いていないと、利用者が
        // 入力方式を入にしたことに気づけない** (ADR-0012)。
        let sink: IUnknown = self.to_interface();
        let open_close_cookie = compartment::advise_open_close(&thread_manager, &sink);
        if open_close_cookie.is_none() {
            log::error("入切の変化を知らせてもらえない");
        }

        *self.this.activation.borrow_mut() = Some(Activation {
            client_id,
            keystrokes,
            thread_manager,
            indicator,
            indicator_object,
            open_close_cookie,
        });

        // 見え方の番号を取る。取れなくても入力は続くので、記録だけする。
        match crate::display::Atoms::register() {
            Ok(atoms) => *self.this.atoms.borrow_mut() = Some(atoms),
            Err(e) => log::error(&format!("表示属性を登録できません: {}", e.message())),
        }

        // 切られていたら、入にする。**SKK では入が常態** (ADR-0013) で、
        // 切ったままでは `Ctrl+J` すら届かない。入って半角英数なら、打鍵の
        // 意味は切のときと変わらないので、邪魔にもならない。
        //
        // ここは入力方式として選ばれた瞬間にしか通らない。焦点が移るたびに
        // これをやると、**利用者が切ったものを勝手に入れ直す**ことになる。
        if let Some(thread_manager) = self.this.thread_manager()
            && !compartment::is_open(&thread_manager)
        {
            log::write("切られていたので入にする");
            compartment::set_open(&thread_manager, client_id, true);
        }

        // 入っているなら、そのモードを掲示する。切なら何も言わない。
        self.this.sync_with_open_state();
        Ok(())
    }

    /// 打鍵を処理し、確定した文字列を文書へ入れる。
    ///
    /// 戻り値は「この打鍵を食べたか」。食べなかった打鍵はアプリへ渡る。
    fn handle_key(&self, context: Ref<ITfContext>, wparam: WPARAM) -> BOOL {
        if !self.this.accepts_keys() {
            return false.into();
        }
        let translated = keys::translate(wparam);
        keys::log_translation(wparam, translated);
        let Some(key) = translated else {
            return false.into();
        };
        let Some(client_id) = self.this.client_id() else {
            return false.into();
        };
        let Some(context) = context.as_ref() else {
            log::trace("文脈がないので素通しする");
            return false.into();
        };

        let response = self.this.engine.borrow_mut().press(key);
        // 組み立てる前に段階を見る。記録しないと決まっているなら、
        // 打鍵のたびに文字列を作る手間も要らない。
        if log::tracing() {
            log::trace(&format!(
                "打鍵 {key:?} → 食べた:{} 確定:{:?} 未確定:{:?}",
                response.handled,
                response.commit,
                response.preedit.display()
            ));
        }
        self.this.apply_events(&response.events);

        self.this.show_mode();
        log::trace("モードを映した");

        // 見え方を今の状態に合わせる。書けなくても、エンジンの状態は
        // もう進んでいる。ここで慌てても直せないので、食べたことだけは
        // 正しく伝える。
        //
        // **辞書登録中は文書に何も書かない。** 登録語として打っている文字は
        // 登録の枠に溜まるものであって、文書に入るものではない。途中の
        // ローマ字だけが文書に現れては、どこへ打っているのか分からなくなる。
        //
        // 区切りごとに見え方の番号を添える。**どう見せるかは
        // [`crate::display`] が決め、貼るのは [`crate::edit`] がやる。**
        let preedit: Vec<(u32, String)> = if response.preedit.registering.is_some() {
            Vec::new()
        } else {
            let atoms = *self.this.atoms.borrow();
            crate::display::document_segments(&response.preedit.segments, atoms)
        };
        let sink: ITfCompositionSink = self.to_interface();
        // 借用を編集セッションより長く持たない。呼んだ先から戻って
        // こられると、借用が重なってパニックになる。
        let carried = self.this.composition.borrow_mut().take().map(|(_, c)| c);

        match edit::update(
            context,
            client_id,
            &sink,
            &response.commit,
            &preedit,
            carried,
        ) {
            Ok(applied) => {
                *self.this.composition.borrow_mut() =
                    applied.composition.map(|c| (context.clone(), c));
                log::trace("文書へ反映した");
                self.this.show_window(applied.extent, applied.owner);
            }
            Err(e) => {
                log::error(&format!("文書へ反映できなかった: {}", e.message()));
                // 書けていない以上、窓だけ残しても嘘になる。
                self.this.candidates.hide();
            }
        }
        response.handled.into()
    }

    /// 打鍵を食べるかどうかだけを答える。状態は変えない。
    fn would_handle_key(&self, wparam: WPARAM) -> BOOL {
        if !self.this.accepts_keys() {
            return false.into();
        }
        let translated = keys::translate(wparam);
        let Some(key) = translated else {
            // 食べないと答えた打鍵は `OnKeyDown` に来ないので、
            // 解釈の記録はここでしか残せない。
            keys::log_translation(wparam, translated);
            return false.into();
        };
        let handled = self.this.engine.borrow().would_handle(key);
        if !handled {
            keys::log_translation(wparam, translated);
        }
        handled.into()
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> Result<()> {
        guard("Activate", || self.activate(ptim, tid))
    }

    fn Deactivate(&self) -> Result<()> {
        guard("Deactivate", || self.this.deactivate())
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    /// `dwflags` は入力先の種類 (`TF_TMF_*`) を伝える。まだ使わない。
    fn ActivateEx(&self, ptim: Ref<ITfThreadMgr>, tid: u32, _dwflags: u32) -> Result<()> {
        guard("ActivateEx", || self.activate(ptim, tid))
    }
}

impl ITfCompositionSink_Impl for TextService_Impl {
    /// composition がこちらの意図と関係なく終わった。
    ///
    /// アプリが文書を触ったときなどに来る。いまは開きっぱなしにしないので
    /// 起きにくいが、来たら入力の途中経過は捨てる。
    fn OnCompositionTerminated(
        &self,
        _ecwrite: u32,
        _pcomposition: Ref<ITfComposition>,
    ) -> Result<()> {
        guard("OnCompositionTerminated", || {
            log::write("composition が外から終わらされた");
            // もう閉じられているので、こちらで片付ける必要はない。
            self.this.composition.borrow_mut().take();
            self.this.engine.borrow_mut().reset();
            Ok(())
        })
    }
}

impl ITfDisplayAttributeProvider_Impl for TextService_Impl {
    /// 名乗る見え方を並べて渡す。
    ///
    /// アプリはこれを見て、貼られた番号が何を意味するかを知る。
    fn EnumDisplayAttributeInfo(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        guard("EnumDisplayAttributeInfo", || {
            let list = ComObject::new(crate::display::AttributeEnum::default());
            Ok(list.to_interface())
        })
    }

    /// 一つを名指しで渡す。
    // TSF が決めた形なので、生のポインタを受けるしかない。中では
    // 確かめてから使う。
    #[allow(
        clippy::not_unsafe_ptr_arg_deref,
        reason = "COM の口の形が決まっている"
    )]
    fn GetDisplayAttributeInfo(&self, guid: *const GUID) -> Result<ITfDisplayAttributeInfo> {
        guard("GetDisplayAttributeInfo", || {
            // SAFETY: TSF が渡す GUID への参照で、この呼び出しの間は有効。
            let Some(guid) = (unsafe { guid.as_ref() }) else {
                return Err(E_INVALIDARG.into());
            };
            let Some(attribute) = crate::display::by_guid(guid) else {
                return Err(E_INVALIDARG.into());
            };
            let info = ComObject::new(crate::display::AttributeInfo::new(attribute));
            Ok(info.to_interface())
        })
    }
}

impl ITfCompartmentEventSink_Impl for TextService_Impl {
    /// 見張っている区画の値が変わった。
    ///
    /// 書いたのが誰かは分からない。利用者かもしれないし、アプリかもしれない。
    /// **こちらが書いた値が戻ってくることもある。** どれであっても、読んで
    /// 合わせるだけでよい。
    // TSF が決めた形なので、生のポインタを受けるしかない。中では
    // `as_ref` で確かめてから使う。
    #[allow(
        clippy::not_unsafe_ptr_arg_deref,
        reason = "COM の口の形が決まっている"
    )]
    fn OnChange(&self, rguid: *const GUID) -> Result<()> {
        guard("OnChange", || {
            // SAFETY: TSF が渡す GUID への参照で、この呼び出しの間は有効。
            let Some(guid) = (unsafe { rguid.as_ref() }) else {
                return Ok(());
            };
            if *guid == GUID_COMPARTMENT_KEYBOARD_OPENCLOSE {
                self.this.sync_with_open_state();
            }
            Ok(())
        })
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    /// 入力先が変わった。入力の途中経過は続けようがないので捨てる。
    fn OnSetFocus(&self, _fforeground: BOOL) -> Result<()> {
        guard("OnSetFocus", || {
            self.this.drop_composition();
            self.this.engine.borrow_mut().reset();
            Ok(())
        })
    }

    fn OnTestKeyDown(
        &self,
        _pic: Ref<ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        guard("OnTestKeyDown", || Ok(self.would_handle_key(wparam)))
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        guard("OnKeyDown", || Ok(self.handle_key(pic, wparam)))
    }

    /// 離した打鍵は使わない。押した側だけで足りる。
    fn OnTestKeyUp(&self, _pic: Ref<ITfContext>, _wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        guard("OnTestKeyUp", || Ok(false.into()))
    }

    fn OnKeyUp(&self, _pic: Ref<ITfContext>, _wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        guard("OnKeyUp", || Ok(false.into()))
    }

    /// 入切のキーが押された。
    ///
    /// 押されたことをここで受け、区画を書き換える。こちらの状態はその
    /// 変化の通知を聞いて合わせる。**二箇所で状態を持たないよう、書いた
    /// ものを読み直す。**
    // TSF が決めた形なので、生のポインタを受けるしかない。中では
    // `as_ref` で確かめてから使う。
    #[allow(
        clippy::not_unsafe_ptr_arg_deref,
        reason = "COM の口の形が決まっている"
    )]
    fn OnPreservedKey(&self, _pic: Ref<ITfContext>, rguid: *const GUID) -> Result<BOOL> {
        guard("OnPreservedKey", || {
            // SAFETY: TSF が渡す GUID への参照で、この呼び出しの間は有効。
            let Some(guid) = (unsafe { rguid.as_ref() }) else {
                return Ok(false.into());
            };
            let Some(thread_manager) = self.this.thread_manager() else {
                return Ok(false.into());
            };
            let Some(client_id) = self.this.client_id() else {
                return Ok(false.into());
            };

            let wanted = if *guid == GUID_PRESERVED_KEY_ON {
                true
            } else if *guid == GUID_PRESERVED_KEY_OFF {
                false
            } else {
                return Ok(false.into());
            };

            if compartment::is_open(&thread_manager) != wanted {
                log::write(&format!(
                    "入切のキーで{}にする",
                    if wanted { "入" } else { "切" }
                ));
                compartment::set_open(&thread_manager, client_id, wanted);
            }
            Ok(true.into())
        })
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
