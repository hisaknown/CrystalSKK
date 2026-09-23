//! SKK の変換状態機械。
//!
//! キーを一つ受け取り、[`Response`] を一つ返す。応答には「アプリへ確定する
//! 文字列」「未確定の表示」「候補ウィンドウの内容」「外部へ伝える副作用」が
//! 含まれる。エンジン自身は I/O を行わない。
//!
//! 状態は三つしかない。辞書登録はこれらと並ぶ第四の状態ではなく、直接入力の
//! 上に積まれた枠として表現している。詳しくは ADR-0002 を参照。

use crate::dict::{Candidate, CandidateSource, Context, EmptyDict, NoopRanker, Query, Ranker};
use crate::kana;
use crate::key::Key;
use crate::mode::InputMode;
use crate::romaji::{RomajiConverter, RomajiTable};

/// 未確定文字列に付く印。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Marker {
    /// 印なし。
    #[default]
    None,
    /// 見出し語入力中 (`▽`)。
    Composing,
    /// 候補選択中 (`▼`)。
    Selecting,
}

impl Marker {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Composing => "▽",
            Self::Selecting => "▼",
        }
    }
}

/// 未確定の表示を成す一区切りの役目。
///
/// 表示属性 (下線の引き方) はこれごとに変わる。**一本の文字列にしてしまうと、
/// どこからが送り仮名かを外から言えない。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// `▽` `▼` の印。状態を表す。
    Marker,
    /// 見出し語と送り仮名の区切り (`*`)。
    ///
    /// 印とは別の役目である。**状態を表すのではなく、境を示す。** 片方だけ
    /// を出す、という選び方ができるように分けてある (ADR-0018)。
    Separator,
    /// 見出し語。打っている最中のもの。
    Midashi,
    /// 送り仮名。区切りの `*` を含む。
    Okuri,
    /// 選ばれている候補。
    Candidate,
}

/// 未確定の表示の一区切り。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub role: Role,
    pub text: String,
}

impl Segment {
    fn new(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
        }
    }
}

/// 未確定の表示状態。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preedit {
    pub marker: Marker,
    /// 印を含めた区切りの並び。
    ///
    /// 繋げれば表示文字列になる ([`Self::display`])。分かれているのは、
    /// **区切りごとに違う表示属性を付けるため**である。
    pub segments: Vec<Segment>,
    /// 辞書登録中なら、登録しようとしている辞書キー。
    pub registering: Option<String>,
}

impl Preedit {
    pub fn is_empty(&self) -> bool {
        self.segments.iter().all(|s| s.text.is_empty())
            && self.marker == Marker::None
            && self.registering.is_none()
    }

    /// 印を含めた表示文字列。
    pub fn display(&self) -> String {
        self.segments.iter().map(|s| s.text.as_str()).collect()
    }
}

/// 見出し語と送り仮名の区切りに出す印。
pub const OKURI_MARK: &str = "*";

/// 候補一覧を出すまでの変換回数。
///
/// SKK は候補を**まず一つずつ**見せる。ほとんどの変換は最初の候補で
/// 決まるので、そのたびに一覧を開いては目障りなだけである。何度か送って
/// 決まらないとき、初めて一覧に頼る。
///
/// 既定は CorvusSKK に合わせて 5。5 回目の変換から一覧に移る。
pub const UNTIL_CANDIDATE_LIST: usize = 5;

/// 一覧から候補を選ぶキー。
///
/// 並び順がそのままラベルの並びになり、**この数が一度に出す候補の数**に
/// なる。選べない候補を並べても仕方がないので、一覧は必ずこの長さで切る。
pub const SELECTION_KEYS: [char; 7] = ['a', 's', 'd', 'f', 'j', 'k', 'l'];

/// 一覧の一ページに出す候補の数。
pub const PAGE_SIZE: usize = SELECTION_KEYS.len();

/// 一覧に移る最初の候補の位置。
const FIRST_LISTED: usize = UNTIL_CANDIDATE_LIST - 1;

/// 候補ウィンドウに出す内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateView {
    pub candidates: Vec<Candidate>,
    /// 選択中の候補の位置。
    ///
    /// 一覧を出している間は、そのページの先頭を指す。ページの中に
    /// 「いま選ばれている一つ」は無く、選ぶのはラベルキーを押すことである。
    pub index: usize,
    /// 送り仮名。表示に付ける。
    pub okuri: Option<String>,
    /// 一覧を出す段階に入っているか。
    ///
    /// **候補ウィンドウを出してよいかどうかがこれで決まる。** 入っていない
    /// うちは `▼` のところに一つ出ているだけで、窓は要らない。
    pub listing: bool,
}

impl CandidateView {
    /// いま出すページの候補。ラベルと対にして返す。
    ///
    /// 一覧を出していないときは空。窓に出すものが無いという意味になる。
    pub fn page(&self) -> Vec<(char, &Candidate)> {
        if !self.listing {
            return Vec::new();
        }
        let start = page_start(self.index);
        self.candidates[start..]
            .iter()
            .take(PAGE_SIZE)
            .zip(SELECTION_KEYS)
            .map(|(candidate, label)| (label, candidate))
            .collect()
    }

    /// 一覧に載る候補。
    ///
    /// **最初の数件は載らない。** そこは一つずつ見せる段階で、一覧には
    /// 現れない。外へ渡すときにこれを混ぜると、ページの区切りが合わなく
    /// なる。
    pub fn listed(&self) -> &[Candidate] {
        self.candidates.get(FIRST_LISTED..).unwrap_or(&[])
    }

    /// いま何ページ目か。0 から数える。
    pub fn page_number(&self) -> usize {
        if !self.listing {
            return 0;
        }
        (page_start(self.index) - FIRST_LISTED) / PAGE_SIZE
    }

    /// 一覧に載る候補は全部で何ページ分あるか。
    pub fn page_count(&self) -> usize {
        self.candidates
            .len()
            .saturating_sub(FIRST_LISTED)
            .div_ceil(PAGE_SIZE)
    }
}

/// この位置を含むページの先頭。
fn page_start(index: usize) -> usize {
    if index < FIRST_LISTED {
        return index;
    }
    FIRST_LISTED + (index - FIRST_LISTED) / PAGE_SIZE * PAGE_SIZE
}

/// 辞書登録中の様子。
///
/// 登録は枠を積んで表す (ADR-0002) ので、入れ子になりうる。ここに出すのは
/// **一番内側の枠**で、`depth` がいくつ積まれているかを示す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationView {
    /// 登録しようとしている見出し語。
    pub key: String,
    /// 送り仮名。あれば見出しに添えて見せる。
    pub okuri: Option<String>,
    /// これまでに溜まった語。
    pub buffer: String,
    /// 積まれている枠の数。1 なら入れ子ではない。
    pub depth: usize,
}

/// エンジンの外側へ伝える副作用。
///
/// ユーザー辞書への書き込みはサーバープロセスの仕事なので、エンジンは
/// 「何が起きたか」を伝えるだけで、自分では永続化しない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// 候補を確定した。出現順の学習に使う。
    Learn { query: Query, word: String },
    /// 新しい語を辞書に登録した。
    Register { query: Query, word: String },
}

/// 一打鍵に対する応答。
#[derive(Debug, Clone, Default)]
pub struct Response {
    /// エンジンがこのキーを処理したか。`false` ならアプリへ素通しする。
    pub handled: bool,
    /// アプリへ確定入力する文字列。
    pub commit: String,
    /// 未確定の表示状態。
    pub preedit: Preedit,
    /// 候補選択中なら候補ウィンドウの内容。
    pub candidates: Option<CandidateView>,
    /// 外部へ伝える副作用。
    pub events: Vec<Event>,
}

/// 見出し語入力中の状態。
#[derive(Debug, Clone, Default)]
struct Composing {
    /// 見出し語。かなモードではひらがなで保持する。
    midashi: String,
    /// 送り仮名。入力が始まっていなければ `None`。
    okuri: Option<Okuri>,
    /// abbrev (`/`) で始まった入力か。この間はローマ字変換を通さない。
    abbrev: bool,
}

/// 送り仮名の入力状態。
#[derive(Debug, Clone)]
struct Okuri {
    /// 送り仮名の最初のローマ字。辞書キーの末尾になる。
    head: char,
    /// これまでに確定した送り仮名のかな。
    kana: String,
}

/// 候補選択中の状態。
#[derive(Debug, Clone)]
struct Selecting {
    query: Query,
    candidates: Vec<Candidate>,
    index: usize,
    /// 候補選択を取りやめたときに戻る先。
    origin: Composing,
}

impl Selecting {
    /// 一覧を出す段階に入っているか。
    fn listing(&self) -> bool {
        self.index >= FIRST_LISTED
    }

    /// いま出しているページの先頭。
    fn page_start(&self) -> usize {
        page_start(self.index)
    }
}

/// 辞書登録の枠。
///
/// 登録中の入力は通常の直接入力とまったく同じに振る舞い、確定した文字列だけが
/// アプリではなくこの枠に溜まる。枠は積めるので、登録中にさらに登録が起きても
/// そのまま入れ子になる。
#[derive(Debug, Clone)]
struct Registration {
    query: Query,
    /// 登録をやめたときに戻る先。
    origin: Composing,
    /// 登録を取りやめたときに戻る、候補選択の状態。
    ///
    /// 辞書を引いて一件も無かったときは候補選択を経ていないので `None`。
    /// **取りやめは「直前へ戻る」であり、直前は候補選択の最後である。**
    resume: Option<Selecting>,
    /// 登録語として溜まった文字列。
    buffer: String,
}

/// 入力状態。
#[derive(Debug, Clone, Default)]
enum State {
    /// 直接入力 (`■`)。
    #[default]
    Direct,
    /// 見出し語入力 (`▽`)。
    Composing(Composing),
    /// 候補選択 (`▼`)。
    Selecting(Selecting),
}

/// 一打鍵の処理中に溜めていく出力。
#[derive(Default)]
struct Out {
    handled: bool,
    commit: String,
    events: Vec<Event>,
}

/// SKK 変換エンジン。
pub struct Engine {
    mode: InputMode,
    romaji: RomajiConverter,
    state: State,
    registrations: Vec<Registration>,
    dict: Box<dyn CandidateSource>,
    ranker: Box<dyn Ranker>,
    context: Context,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("mode", &self.mode)
            .field("state", &self.state)
            .field("registrations", &self.registrations.len())
            .field("pending", &self.romaji.pending())
            .finish_non_exhaustive()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(Box::new(EmptyDict))
    }
}

impl Engine {
    pub fn new(dict: Box<dyn CandidateSource>) -> Self {
        Self {
            mode: InputMode::Hiragana,
            romaji: RomajiConverter::default(),
            state: State::Direct,
            registrations: Vec::new(),
            dict,
            ranker: Box::new(NoopRanker),
            context: Context::default(),
        }
    }

    pub fn with_ranker(mut self, ranker: Box<dyn Ranker>) -> Self {
        self.ranker = ranker;
        self
    }

    pub fn with_romaji_table(mut self, table: RomajiTable) -> Self {
        self.romaji = RomajiConverter::new(table);
        self
    }

    pub fn mode(&self) -> InputMode {
        self.mode
    }

    /// 変換時に参照される周辺情報を差し替える。TSF 側が毎打鍵の前に更新する。
    pub fn set_context(&mut self, context: Context) {
        self.context = context;
    }

    /// 辞書登録の入れ子の深さ。0 なら登録中ではない。
    pub fn registration_depth(&self) -> usize {
        self.registrations.len()
    }

    /// 辞書登録中なら、その一番内側の様子。
    ///
    /// 登録中の入力は**文書には入らない**。確定した文字列はこの枠に溜まる
    /// ので、利用者に見せるにはここから取る必要がある。
    pub fn registration(&self) -> Option<RegistrationView> {
        let frame = self.registrations.last()?;
        Some(RegistrationView {
            key: frame.query.key.clone(),
            okuri: frame.query.okuri.clone(),
            buffer: frame.buffer.clone(),
            depth: self.registrations.len(),
        })
    }

    /// 入力の途中経過をすべて捨て、直接入力に戻す。
    ///
    /// 入力先が変わったときのように、続きを入力しようがない場面で呼ぶ。
    /// 未確定の文字列は失われる。
    ///
    /// 入力モードは保たれる。モードは利用者が選んだ設定であり、入力の
    /// 途中経過ではないため。
    pub fn reset(&mut self) {
        self.state = State::Direct;
        self.registrations.clear();
        self.romaji.clear();
    }

    /// 入力モードを入れ替え、入力の途中経過を捨てる。
    ///
    /// 入力方式が入にされたときなど、**外の都合で状態を作り直す**ための口。
    /// 打鍵で移るモード変更 (`q` や `l`) はエンジンの中で完結するので、
    /// これを通らない。
    pub fn restart_in(&mut self, mode: InputMode) {
        self.reset();
        self.mode = mode;
    }

    /// このキーをエンジンが処理するか。状態を変えずに答える。
    ///
    /// TSF は「食べるか」を先に尋ねてから実際に渡してくる
    /// (`OnTestKeyDown` → `OnKeyDown`)。二つの答えが食い違うと打鍵が
    /// 消えたり二重に入ったりするので、[`Self::press`] が返す `handled` と
    /// 必ず一致していなければならない。一致は試験で担保している。
    pub fn would_handle(&self, key: Key) -> bool {
        match &self.state {
            State::Direct => self.would_handle_direct(key),
            State::Composing(comp) => would_handle_composing(comp, key),
            State::Selecting(_) => would_handle_selecting(key),
        }
    }

    fn would_handle_direct(&self, key: Key) -> bool {
        // ひらがなへ戻す操作だけは、どのモードでも受け取る。
        if key == Key::Ctrl('j') {
            return true;
        }
        let registering = !self.registrations.is_empty();
        if !self.mode.is_kana() {
            return match key {
                // 登録中は英数モードでも打鍵を受け取る。**登録語を打って
                // いるのだから、アプリへ抜けては困る。**
                Key::Char(_) | Key::Space => self.mode == InputMode::FullAscii || registering,
                Key::Enter | Key::Backspace | Key::Escape | Key::Ctrl('g') => registering,
                _ => false,
            };
        }
        match key {
            Key::Char(_) | Key::Space | Key::Ctrl('g') | Key::Ctrl('q') => true,
            // 登録を取りやめるときだけ受け取る。
            Key::Escape => registering,
            Key::Ctrl(_) | Key::Tab | Key::Up | Key::Down => false,
            // 未確定を確定させるとき、または辞書登録を終えるときだけ受け取る。
            Key::Enter => !self.registrations.is_empty() || self.romaji.pending_kana().is_some(),
            // 消すものがあるときだけ受け取る。
            Key::Backspace => {
                !self.romaji.is_empty()
                    || self
                        .registrations
                        .last()
                        .is_some_and(|r| !r.buffer.is_empty())
            }
        }
    }

    /// 一打鍵を処理する。
    pub fn press(&mut self, key: Key) -> Response {
        let mut out = Out {
            handled: true,
            ..Out::default()
        };
        let state = std::mem::take(&mut self.state);
        match state {
            State::Direct => self.on_direct(key, &mut out),
            State::Composing(c) => self.on_composing(c, key, &mut out),
            State::Selecting(s) => self.on_selecting(s, key, &mut out),
        }
        self.context.mode = self.mode;

        Response {
            handled: out.handled,
            commit: out.commit,
            preedit: self.preedit(),
            candidates: self.candidates(),
            events: out.events,
        }
    }

    /// 現在の未確定表示。
    pub fn preedit(&self) -> Preedit {
        let registering = self.registrations.last().map(|r| r.query.key.clone());
        match &self.state {
            State::Direct => Preedit {
                marker: Marker::None,
                segments: vec![Segment::new(Role::Midashi, self.romaji.pending())],
                registering,
            },
            State::Composing(c) => {
                let mut segments = vec![
                    Segment::new(Role::Marker, Marker::Composing.prefix()),
                    Segment::new(Role::Midashi, c.midashi.clone()),
                ];
                match &c.okuri {
                    // 送り仮名を打っている最中。打ちかけのローマ字も
                    // 送り仮名の側に付く。
                    //
                    // 区切りの `*` は**印として独立させる**。印をどう見せるか
                    // は front end の裁量で、空白に置き換えることもできる
                    // (PRD Q-09)。埋め込んでしまうと、その選択を奪う。
                    Some(okuri) => {
                        segments.push(Segment::new(Role::Separator, OKURI_MARK));
                        segments.push(Segment::new(
                            Role::Okuri,
                            format!("{}{}", okuri.kana, self.romaji.pending()),
                        ));
                    }
                    None => {
                        if let Some(last) = segments.last_mut() {
                            last.text.push_str(self.romaji.pending());
                        }
                    }
                }
                Preedit {
                    marker: Marker::Composing,
                    segments,
                    registering,
                }
            }
            State::Selecting(s) => {
                let candidate = &s.candidates[s.index];
                let mut segments = vec![
                    Segment::new(Role::Marker, Marker::Selecting.prefix()),
                    Segment::new(Role::Candidate, candidate.word.clone()),
                ];
                // 送り仮名は候補の後ろに付く。**確定しているので、
                // 打っている最中の見出し語とは見え方を変えたい。**
                if let Some(okuri) = &s.query.okuri {
                    segments.push(Segment::new(Role::Okuri, okuri.clone()));
                }
                Preedit {
                    marker: Marker::Selecting,
                    segments,
                    registering,
                }
            }
        }
    }

    /// 候補選択中なら候補ウィンドウの内容。それ以外は `None`。
    pub fn candidates(&self) -> Option<CandidateView> {
        match &self.state {
            State::Selecting(s) => Some(CandidateView {
                candidates: s.candidates.clone(),
                index: s.index,
                okuri: s.query.okuri.clone(),
                listing: s.listing(),
            }),
            _ => None,
        }
    }

    /// 確定した文字列を送り出す。
    ///
    /// 辞書登録中なら、アプリではなく登録の枠に溜まる。
    fn emit(&mut self, text: &str, out: &mut Out) {
        if text.is_empty() {
            return;
        }
        match self.registrations.last_mut() {
            Some(reg) => reg.buffer.push_str(text),
            None => out.commit.push_str(text),
        }
        self.context.recent_commits.insert(0, text.to_owned());
        self.context.recent_commits.truncate(RECENT_COMMITS);
    }

    // --- 直接入力 ------------------------------------------------------

    fn on_direct(&mut self, key: Key, out: &mut Out) {
        self.state = State::Direct;

        if let Key::Ctrl('j') = key {
            let rest = self.romaji.flush();
            self.emit(&self.mode.render_kana(&rest).clone(), out);
            self.mode = InputMode::Hiragana;
            return;
        }

        if !self.mode.is_kana() {
            self.on_direct_ascii(key, out);
            return;
        }

        match key {
            Key::Ctrl('g') => {
                // 打ちかけのローマ字が残っていれば、まずそれを捨てる。
                // 何も残っていないなら、登録そのものを取りやめる。
                //
                // 登録中でないときは、捨てるものが無くても打鍵は食べる。
                // `Ctrl+G` は SKK の「取り消し」であって、アプリへ渡して
                // よいキーではない。
                if self.romaji.is_empty() && !self.registrations.is_empty() {
                    self.cancel_registration(out);
                } else {
                    self.romaji.clear();
                }
            }
            Key::Escape => self.cancel_registration(out),
            Key::Ctrl('q') => {
                self.flush_romaji(out);
                self.mode = match self.mode {
                    InputMode::HalfKatakana => InputMode::Hiragana,
                    _ => InputMode::HalfKatakana,
                };
            }
            Key::Char('l') => {
                self.flush_romaji(out);
                self.mode = InputMode::Ascii;
            }
            Key::Char('L') => {
                self.flush_romaji(out);
                self.mode = InputMode::FullAscii;
            }
            Key::Char('q') => {
                self.flush_romaji(out);
                // ひらがなからはカタカナへ。それ以外のかなモードからは
                // ひらがなへ戻す。`q` はいつでも素のかな入力に帰る手段になる。
                self.mode = match self.mode {
                    InputMode::Hiragana => InputMode::Katakana,
                    _ => InputMode::Hiragana,
                };
            }
            Key::Char('/') => {
                self.romaji.clear();
                self.state = State::Composing(Composing {
                    abbrev: true,
                    ..Composing::default()
                });
            }
            Key::Char(c) if c.is_ascii_uppercase() => {
                self.settle_before_shift(out);
                let mut comp = Composing::default();
                let kana = self.romaji.feed(c.to_ascii_lowercase());
                comp.midashi.push_str(&kana);
                self.state = State::Composing(comp);
            }
            Key::Char(c) => {
                let kana = self.romaji.feed(c);
                let rendered = self.mode.render_kana(&kana);
                self.emit(&rendered, out);
            }
            Key::Space => {
                self.flush_romaji(out);
                self.emit(" ", out);
            }
            Key::Enter => {
                if self.registrations.is_empty() {
                    let before = out.commit.len();
                    self.flush_romaji(out);
                    // 未確定を確定させただけなら改行は送らない。何もなければ
                    // 改行はアプリの仕事なので素通しする。
                    out.handled = out.commit.len() != before;
                } else {
                    self.flush_romaji(out);
                    self.finish_registration(out);
                }
            }
            Key::Backspace => {
                if self.romaji.backspace() {
                    return;
                }
                match self.registrations.last_mut() {
                    Some(reg) => {
                        if reg.buffer.pop().is_none() {
                            out.handled = false;
                        }
                    }
                    None => out.handled = false,
                }
            }
            Key::Tab | Key::Up | Key::Down | Key::Ctrl(_) => out.handled = false,
        }
    }

    /// 辞書登録を取りやめ、直前の状態へ戻す。
    ///
    /// 戻る先は**候補選択の最後**である。候補を送り切って登録に入ったの
    /// だから、取りやめれば送り切る前に立っていた場所へ返るのが素直で、
    /// 見出し語入力まで巻き戻すのは一段行き過ぎになる。辞書に一件も
    /// 無くて登録に入ったときだけ、戻る先が見出し語入力になる。
    ///
    /// 一番内側の枠だけを畳む。入れ子になっているなら、外側の登録は
    /// 続いている。**「直前に戻る」であって「全部やめる」ではない。**
    ///
    /// 登録中でなければ何もせず、打鍵はアプリへ渡す。
    fn cancel_registration(&mut self, out: &mut Out) {
        let Some(frame) = self.registrations.pop() else {
            out.handled = false;
            return;
        };
        self.romaji.clear();
        self.state = match frame.resume {
            Some(selecting) => State::Selecting(selecting),
            None => State::Composing(frame.origin),
        };
    }

    /// 英数モードの直接入力。かな変換を通さない。
    fn on_direct_ascii(&mut self, key: Key, out: &mut Out) {
        match (self.mode, key) {
            (InputMode::FullAscii, Key::Char(c)) => {
                let text = kana::to_fullwidth_ascii(&c.to_string());
                self.emit(&text, out);
            }
            (InputMode::FullAscii, Key::Space) => self.emit("　", out),
            // 登録中は半角英数でも打鍵を受け取り、登録語に溜める。
            (InputMode::Ascii, Key::Char(c)) if !self.registrations.is_empty() => {
                self.emit(&c.to_string(), out);
            }
            (InputMode::Ascii, Key::Space) if !self.registrations.is_empty() => {
                self.emit(" ", out);
            }
            (_, Key::Enter) if !self.registrations.is_empty() => {
                self.finish_registration(out);
            }
            (_, Key::Backspace) if !self.registrations.is_empty() => {
                if let Some(frame) = self.registrations.last_mut()
                    && frame.buffer.pop().is_none()
                {
                    out.handled = false;
                }
            }
            (_, Key::Escape | Key::Ctrl('g')) if !self.registrations.is_empty() => {
                self.cancel_registration(out);
            }
            _ => out.handled = false,
        }
    }

    /// シフト付きの打鍵が来たとき、その前の未確定打鍵を始末する。
    ///
    /// 単独でかなになる打鍵 (`n` → `ん`) はここで確定させる。ならない打鍵
    /// (`k` など) は**残す**。残せば続く打鍵と組み合わさり、見出し語の
    /// 一文字目になる。
    ///
    /// これは打ち間違いの救済である。`KayoU` と打つべきところを `kAyoU` と
    /// 打ってしまっても、`k` が捨てられずに `か` となり `▽かよ` から続けられる。
    /// 正しく打たれた入力では、シフトの時点で未確定打鍵は空か `n` しかないので、
    /// 挙動は変わらない。
    fn settle_before_shift(&mut self, out: &mut Out) {
        let Some(kana) = self.romaji.take_pending_kana() else {
            return;
        };
        let rendered = self.mode.render_kana(&kana);
        self.emit(&rendered, out);
    }

    fn flush_romaji(&mut self, out: &mut Out) {
        let rest = self.romaji.flush();
        if rest.is_empty() {
            return;
        }
        let rendered = self.mode.render_kana(&rest);
        self.emit(&rendered, out);
    }

    // --- 見出し語入力 --------------------------------------------------

    fn on_composing(&mut self, mut comp: Composing, key: Key, out: &mut Out) {
        match key {
            Key::Ctrl('g') => {
                self.romaji.clear();
                self.state = State::Direct;
            }
            Key::Ctrl('j') | Key::Enter => {
                self.absorb_pending(&mut comp);
                let text = self.commit_text_of(&comp);
                self.emit(&text, out);
                self.state = State::Direct;
            }
            Key::Char('q') if !comp.abbrev => {
                self.absorb_pending(&mut comp);
                let text = kana::to_katakana(&self.midashi_with_okuri(&comp));
                self.emit(&text, out);
                self.state = State::Direct;
            }
            // `q` が見出し語をカタカナで確定するのと同じく、`C-q` は半角カタカナで
            // 確定する。直接入力での `q` と `C-q` の関係をそのまま写したもの。
            Key::Ctrl('q') if !comp.abbrev => {
                self.absorb_pending(&mut comp);
                let text = kana::to_halfwidth_katakana(&self.midashi_with_okuri(&comp));
                self.emit(&text, out);
                self.state = State::Direct;
            }
            Key::Space => self.convert(comp),
            Key::Backspace => {
                if self.romaji.backspace() {
                    self.state = State::Composing(comp);
                    return;
                }
                match comp.okuri.as_mut() {
                    Some(okuri) => {
                        if okuri.kana.pop().is_none() {
                            comp.okuri = None;
                        }
                    }
                    None => {
                        comp.midashi.pop();
                    }
                }
                if comp.midashi.is_empty() && comp.okuri.is_none() && !comp.abbrev {
                    self.state = State::Direct;
                } else {
                    self.state = State::Composing(comp);
                }
            }
            Key::Char(c) if comp.abbrev => {
                comp.midashi.push(c);
                self.state = State::Composing(comp);
            }
            // シフト付きの打鍵は送り仮名の開始を示す。ただし送り仮名が始まれるのは、
            // 見出し語にかなが一文字でもあり、まだ送り仮名が始まっていないときだけ。
            // それ以外の位置でのシフトは、新しい区切りを作れないので意味を持たない
            // (`KAyoU` のような打ちすぎ) ので、小文字として扱う。
            Key::Char(c)
                if c.is_ascii_uppercase() && comp.okuri.is_none() && !comp.midashi.is_empty() =>
            {
                self.absorb_settled_pending(&mut comp);
                // 単独では成立しない打鍵が残っているなら、それも送り仮名の一部。
                // `TabekU` のように子音を打ってからシフトした場合、送り仮名は
                // `く` であり、辞書キーの末尾はその子音になる。
                let head = self
                    .romaji
                    .pending()
                    .chars()
                    .next()
                    .unwrap_or_else(|| c.to_ascii_lowercase());
                comp.okuri = Some(Okuri {
                    head,
                    kana: String::new(),
                });
                let kana = self.romaji.feed(c.to_ascii_lowercase());
                if let Some(okuri) = comp.okuri.as_mut() {
                    okuri.kana.push_str(&kana);
                }
                self.convert_if_okuri_complete(comp);
            }
            Key::Char(c) => {
                let c = if c.is_ascii_uppercase() {
                    c.to_ascii_lowercase()
                } else {
                    c
                };
                let kana = self.romaji.feed(c);
                match comp.okuri.as_mut() {
                    Some(okuri) => okuri.kana.push_str(&kana),
                    None => comp.midashi.push_str(&kana),
                }
                self.convert_if_okuri_complete(comp);
            }
            Key::Escape => {
                self.romaji.clear();
                self.state = State::Direct;
            }
            Key::Tab | Key::Up | Key::Down | Key::Ctrl(_) => {
                out.handled = false;
                self.state = State::Composing(comp);
            }
        }
    }

    /// 送り仮名の開始時に、その前の未確定打鍵を始末する。
    ///
    /// [`Self::settle_before_shift`] と同じ考え方で、単独で成立する打鍵だけを
    /// 取り込み、成立しないものは残して次の打鍵と組み合わせる。
    fn absorb_settled_pending(&mut self, comp: &mut Composing) {
        let Some(kana) = self.romaji.take_pending_kana() else {
            return;
        };
        match comp.okuri.as_mut() {
            Some(okuri) => okuri.kana.push_str(&kana),
            None => comp.midashi.push_str(&kana),
        }
    }

    /// 未確定のローマ字を見出し語または送り仮名に取り込む。
    fn absorb_pending(&mut self, comp: &mut Composing) {
        let rest = self.romaji.flush();
        if rest.is_empty() {
            return;
        }
        match comp.okuri.as_mut() {
            Some(okuri) => okuri.kana.push_str(&rest),
            None => comp.midashi.push_str(&rest),
        }
    }

    /// 送り仮名が一文字確定したら変換に進む。そうでなければ入力を続ける。
    /// 候補選択をやめ、見出し語入力へ戻す。
    ///
    /// **送り仮名は見出し語に溶かす。** 区切りを残したまま戻ると、続きを
    /// 打つのに一度消さなければならない。`▽な*く` から取り消したとき、
    /// `▽なく` として続けられるほうが素直である。
    ///
    /// ddskk も CorvusSKK も既定でこうする。ddskk の
    /// `skk-delete-okuri-when-quit` は nil が既定で、docstring がそのまま
    /// 書いている — 「▽な*く -> ▼泣く -> C-g -> ▽なく」。
    ///
    /// 送り仮名ごと消す作法もある (ddskk では非 nil、CorvusSKK では
    /// `DelOkuriCncl`)。どちらも既定ではない。
    fn back_to_composing(&mut self, mut comp: Composing) {
        if let Some(okuri) = comp.okuri.take() {
            comp.midashi.push_str(&okuri.kana);
        }
        self.state = State::Composing(comp);
    }

    fn convert_if_okuri_complete(&mut self, comp: Composing) {
        let ready = comp.okuri.as_ref().is_some_and(|o| !o.kana.is_empty());
        if ready && self.romaji.is_empty() {
            self.convert(comp);
        } else {
            self.state = State::Composing(comp);
        }
    }

    fn midashi_with_okuri(&self, comp: &Composing) -> String {
        match &comp.okuri {
            Some(okuri) => format!("{}{}", comp.midashi, okuri.kana),
            None => comp.midashi.clone(),
        }
    }

    /// 見出し語をそのまま確定するときの文字列。
    fn commit_text_of(&self, comp: &Composing) -> String {
        let text = self.midashi_with_okuri(comp);
        if comp.abbrev {
            text
        } else {
            self.mode.render_kana(&text)
        }
    }

    /// 辞書を引き、候補があれば選択へ、なければ登録へ進む。
    fn convert(&mut self, mut comp: Composing) {
        self.absorb_pending(&mut comp);
        self.romaji.clear();

        let query = match &comp.okuri {
            Some(okuri) => Query::okuri_ari(&comp.midashi, okuri.head, okuri.kana.clone()),
            None => Query::okuri_nashi(comp.midashi.clone()),
        };

        let mut candidates = self.dict.lookup(&query);
        self.ranker.rank(&self.context, &query, &mut candidates);

        if candidates.is_empty() {
            if !self.dict.available() {
                // 引けなかっただけで、無いとは限らない。**見出し語入力の
                // まま留める。** ここで登録を始めると、知っているはずの語を
                // 「辞書に無い」と言われたうえ、登録すれば辞書が汚れる。
                self.state = State::Composing(comp);
                return;
            }
            self.start_registration(query, comp, None);
        } else {
            self.state = State::Selecting(Selecting {
                query,
                candidates,
                index: 0,
                origin: comp,
            });
        }
    }

    // --- 候補選択 ------------------------------------------------------

    fn on_selecting(&mut self, mut sel: Selecting, key: Key, out: &mut Out) {
        match key {
            Key::Space | Key::Down => {
                // 一覧を出しているなら、送るのは一件ずつではなく一ページ
                // ずつ。見えているものを送り直しても意味がない。
                let next = if sel.listing() {
                    sel.page_start() + PAGE_SIZE
                } else {
                    sel.index + 1
                };
                if next < sel.candidates.len() {
                    sel.index = next;
                    self.state = State::Selecting(sel);
                } else {
                    // 候補を出し切ったら辞書登録へ。取りやめたときに
                    // 戻れるよう、**最後に見ていたところ**を控えておく。
                    // 一覧が出ていたなら最後のページ、出ていなかったなら
                    // 最後の一件。`page_start` がどちらも言い当てる。
                    let resume = Selecting {
                        index: page_start(sel.candidates.len().saturating_sub(1)),
                        ..sel.clone()
                    };
                    self.start_registration(sel.query, sel.origin, Some(resume));
                }
            }
            Key::Char('x') | Key::Up => {
                if sel.listing() {
                    let start = sel.page_start();
                    sel.index = if start == FIRST_LISTED {
                        // 最初のページから戻るときは、一覧を畳んで一つずつの
                        // 見え方に返る。戻る先は、一覧に移る直前の候補。
                        FIRST_LISTED.saturating_sub(1)
                    } else {
                        start - PAGE_SIZE
                    };
                    self.state = State::Selecting(sel);
                } else if sel.index > 0 {
                    sel.index -= 1;
                    self.state = State::Selecting(sel);
                } else {
                    self.back_to_composing(sel.origin);
                }
            }
            Key::Enter | Key::Ctrl('j') => self.commit_selection(sel, out),
            Key::Ctrl('g') | Key::Backspace | Key::Escape => {
                self.back_to_composing(sel.origin);
            }
            Key::Char(c) if sel.listing() && SELECTION_KEYS.contains(&c) => {
                self.choose_from_page(sel, c, out);
            }
            Key::Char(_) | Key::Ctrl('q') => {
                // 暗黙の確定。確定させた上で、このキーを直接入力として解釈し直す。
                self.commit_selection(sel, out);
                self.on_direct(key, out);
            }
            Key::Tab | Key::Ctrl(_) => {
                out.handled = false;
                self.state = State::Selecting(sel);
            }
        }
    }

    /// 一覧のラベルキーで候補を選ぶ。
    ///
    /// そのラベルに候補が無いときは、何もせず一覧に留まる。**押し間違いで
    /// 関係のない文字が入るより、何も起きないほうがよい。**
    fn choose_from_page(&mut self, mut sel: Selecting, label: char, out: &mut Out) {
        let Some(offset) = SELECTION_KEYS.iter().position(|k| *k == label) else {
            self.state = State::Selecting(sel);
            return;
        };
        let chosen = sel.page_start() + offset;
        if chosen >= sel.candidates.len() {
            self.state = State::Selecting(sel);
            return;
        }
        sel.index = chosen;
        self.commit_selection(sel, out);
    }

    fn commit_selection(&mut self, sel: Selecting, out: &mut Out) {
        let candidate = sel.candidates[sel.index].clone();
        let text = candidate.to_text(sel.query.okuri.as_deref());
        self.emit(&text, out);
        out.events.push(Event::Learn {
            query: sel.query,
            word: candidate.word,
        });
        self.state = State::Direct;
    }

    // --- 辞書登録 ------------------------------------------------------

    fn start_registration(&mut self, query: Query, origin: Composing, resume: Option<Selecting>) {
        self.romaji.clear();
        self.registrations.push(Registration {
            query,
            origin,
            resume,
            buffer: String::new(),
        });
        self.state = State::Direct;
    }

    /// 登録を終える。入力が空なら登録せず、見出し語入力に戻る。
    fn finish_registration(&mut self, out: &mut Out) {
        let Some(reg) = self.registrations.pop() else {
            return;
        };
        if reg.buffer.is_empty() {
            self.state = State::Composing(reg.origin);
            return;
        }
        let text = match &reg.query.okuri {
            Some(okuri) => format!("{}{okuri}", reg.buffer),
            None => reg.buffer.clone(),
        };
        self.emit(&text, out);
        out.events.push(Event::Register {
            query: reg.query,
            word: reg.buffer,
        });
        self.state = State::Direct;
    }
}

/// 見出し語入力中に受け取るキーか。[`Engine::would_handle`] の一部。
fn would_handle_composing(comp: &Composing, key: Key) -> bool {
    match key {
        Key::Ctrl('g') | Key::Ctrl('j') => true,
        Key::Ctrl('q') => !comp.abbrev,
        Key::Ctrl(_) | Key::Tab | Key::Up | Key::Down => false,
        _ => true,
    }
}

/// 候補選択中に受け取るキーか。[`Engine::would_handle`] の一部。
fn would_handle_selecting(key: Key) -> bool {
    match key {
        Key::Ctrl('j') | Key::Ctrl('g') | Key::Ctrl('q') => true,
        Key::Ctrl(_) | Key::Tab => false,
        _ => true,
    }
}

/// 文脈として保持する直近の確定文字列の数。
const RECENT_COMMITS: usize = 8;
