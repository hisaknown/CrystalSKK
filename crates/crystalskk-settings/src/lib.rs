//! 設定ファイル。
//!
//! # ファイルに書かれていることが、すべてである
//!
//! CrystalSKK は既定値を持たない (ADR-0020)。効いている値は必ず利用者の
//! 設定ファイルに書かれている。**利用者の知らないところに設定が生える
//! ことはない。**
//!
//! 項目が足りないとき (新しい版で項目が増えた、利用者が消した) は、
//! 雛形 [`TEMPLATE`] から**ファイルに書き足してから**使う。書き足した
//! 項目にはそのことを注記する。雛形を実行時の既定値として参照することは
//! ない。雛形が使われるのは、ファイルを作るときと、書き足すときだけである。
//!
//! # 書き足しても、利用者の書いたものは崩さない
//!
//! `toml_edit` を使う。利用者のコメントや並びはそのまま残り、足りない
//! 項目だけがその節の中に入る。知らない項目は消さずに知らせるだけにする。
//! 打ち間違いか、廃止された項目か、利用者にしか分からない。
//!
//! # ローマ字テーブルは書き足さない
//!
//! ローマ字テーブルは別のファイルで、**利用者の持ち物**として扱う
//! (ADR-0021)。無ければ雛形から作るが、あれば一切触らない。規則が一つ
//! 無いのは、たいてい利用者が消したからである。
//!
//! # 雛形に戻す
//!
//! どちらのファイルも、利用者が頼めば雛形で上書きする ([`reset_settings`],
//! [`reset_romaji`])。元の中身は隣に退避し、消さない。

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use std::path::PathBuf;

mod keys;

pub use keys::{Hotkey, HotkeyKey, OnOffKeys, clashes};

use crystalskk_core::RomajiTable;
use crystalskk_core::options::{CandidateOptions, CompletionOptions, Options};
use toml_edit::{DocumentMut, Item, Table};

/// 雛形。ファイルを作るときと、足りない項目を書き足すときだけに使う。
pub const TEMPLATE: &str = include_str!("../default.toml");

/// ローマ字テーブルの雛形。ファイルが無いときに作るためだけに使う。
pub const ROMAJI_TEMPLATE: &str = include_str!("../romaji.txt");

/// 書き足した項目に添える注記。
fn added_note() -> String {
    format!(
        "# CrystalSKK {} で書き足しました\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// 設定の一式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// 変換エンジンが使う値。
    pub engine: Options,
    /// 窓の見せ方。
    pub window: Window,
    /// カーソルのそばに出す入力モードの絵。
    pub mode_indicator: ModeIndicator,
    /// 候補の窓と、カーソルのそばの窓の色。
    pub colors: Colors,
    /// 候補の窓と、カーソルのそばの窓の大きさと書体。
    pub popup: Popup,
    /// 引く辞書。並べた順に引く。**使うのは辞書サーバだけ。**
    pub dictionaries: Vec<Source>,
    /// 変換の候補を前後の文章から並べる (ADR-0030)。
    pub ranker: Ranker,
    /// 未確定の印を、文書に書くときの文字 (ADR-0033)。
    pub markers: Markers,
    /// 入切のキー (ADR-0038)。**TIP だけが使う。**
    pub on_off: OnOffKeys,
}

/// 未確定の印を、文書に書くときの文字。**TIP だけが使う。** CLI は
/// 下線を引けないので、記号のまま出す。
///
/// どれも何文字でもよく、空でもよい。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Markers {
    /// 見出し語入力中 (`▽`)。読みがあってもなくても出す。
    pub composing: String,
    /// 見出し語入力中で、読みがまだ無いとき (`▽` だけのとき) に、
    /// `composing` の後ろに足すもの。**読みを消し切っても、変換を
    /// 始めたところが見える。**
    pub composing_empty: String,
    /// 候補選択中 (`▼`)。
    pub selecting: String,
    /// 見出し語と送り仮名の区切り (`*`)。
    pub okuri: String,
}

/// 変換の候補を前後の文章から並べるための設定。
///
/// 前後の文章を読むのは TIP、並べるのは辞書サーバである。両方がこれを使う。
///
/// **言語モデルは選べない。** プログラムの一部として一緒に導入され、置き場所も
/// 決まっている (ADR-0031)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranker {
    /// 並べ替えるか。
    pub enabled: bool,
    /// 辞書の順の重み。
    pub weight: Weight,
    /// 並べ替えを待つ時間 (ミリ秒)。
    pub deadline_ms: u32,
    /// カーソルより前の文章を何文字見せるか。
    pub before: usize,
    /// カーソルより後の文章を何文字見せるか。0 なら見せない。
    pub after: usize,
    /// 辞書の順で上から何個までを並べ替えるか。
    pub top: usize,
    /// 前後の文章に、句読点や空白を除いて何文字あれば並べ替えるか。
    pub min_context: usize,
    /// 採点に使うスレッドの数。
    pub threads: usize,
}

/// 0 以上の重み。
///
/// 設定の一式を比べられるように (変わったときだけ作り直すために)、
/// ビット列で比べる。
#[derive(Debug, Clone, Copy)]
pub struct Weight(pub f32);

impl PartialEq for Weight {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Weight {}

/// 辞書の在りか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// 取得して手元に置く。
    Url(String),
    /// 手元のファイルを直に読む。書かれたままの場所で、相対なら設定
    /// ファイルと同じ場所から解く ([`Source::resolve`])。
    File(String),
    /// 元の辞書に載っているカタカナ語を、語自身の読みで引けるようにした
    /// 辞書 (ADR-0023)。元の辞書は URL かファイル。
    Katakana(Box<Source>),
}

impl Source {
    fn parse(text: &str) -> Self {
        let lowered = text.to_ascii_lowercase();
        if lowered.starts_with("https://") || lowered.starts_with("http://") {
            Self::Url(text.to_owned())
        } else {
            Self::File(text.to_owned())
        }
    }

    /// 元の在りか。派生した辞書なら、元の辞書の在りか。
    pub fn base(&self) -> &Self {
        match self {
            Self::Katakana(from) => from.base(),
            _ => self,
        }
    }

    /// カタカナ語の辞書として作るものか。
    pub fn is_katakana(&self) -> bool {
        matches!(self, Self::Katakana(_))
    }

    /// ファイルなら、設定ファイルの置き場所から見た場所。URL なら `None`。
    pub fn resolve(&self, settings_directory: &Path) -> Option<PathBuf> {
        let Self::File(written) = self.base() else {
            return None;
        };
        let written = Path::new(written);
        Some(if written.is_absolute() {
            written.to_path_buf()
        } else {
            settings_directory.join(written)
        })
    }

    /// 書かれたままの姿。知らせに使う。
    pub fn as_written(&self) -> String {
        match self {
            Self::Url(text) | Self::File(text) => text.clone(),
            Self::Katakana(from) => {
                format!("{{ katakana_from = \"{}\" }}", from.as_written())
            }
        }
    }
}

/// カーソルのそばに出す入力モードの絵。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeIndicator {
    /// モードや入切を切り替えたときに出すか。
    pub on_switch: bool,
    /// 入力先に入ったときにも出すか。
    pub on_focus: bool,
    /// 出しておく時間 (ミリ秒)。
    pub duration_ms: u32,
}

/// 候補の窓と、カーソルのそばの窓の大きさと書体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Popup {
    /// 大きさ (百分率)。モニターの拡大率に掛け合わせる。
    pub scale: u16,
    /// 書体の名前。`None` は Windows のメッセージの書体。
    pub font: Option<String>,
    /// 文字の大きさ (ポイント)。`None` は Windows のメッセージの書体の大きさ。
    pub font_size: Option<u16>,
}

/// 大きさの範囲 (百分率)。
const SCALE_RANGE: std::ops::RangeInclusive<i64> = 50..=300;

/// 文字の大きさの範囲 (ポイント)。
const FONT_SIZE_RANGE: std::ops::RangeInclusive<i64> = 6..=72;

fn popup(table: &Table) -> Result<Popup, Error> {
    let scale = value(table, "popup", "scale")?
        .as_integer()
        .filter(|n| SCALE_RANGE.contains(n))
        .and_then(|n| u16::try_from(n).ok())
        .ok_or_else(|| {
            Error::new(format!(
                "popup.scale は {} から {} の整数で書いてください",
                SCALE_RANGE.start(),
                SCALE_RANGE.end()
            ))
        })?;
    let font = match value(table, "popup", "font")?.as_str() {
        Some("system") => None,
        Some(name) if !name.trim().is_empty() => Some(name.trim().to_owned()),
        _ => {
            return Err(Error::new(
                "popup.font は \"system\" か書体の名前で書いてください (例: \"BIZ UDPGothic\")",
            ));
        }
    };
    let size = value(table, "popup", "font_size")?;
    let font_size = if size.as_str() == Some("system") {
        None
    } else {
        Some(
            size.as_integer()
                .filter(|n| FONT_SIZE_RANGE.contains(n))
                .and_then(|n| u16::try_from(n).ok())
                .ok_or_else(|| {
                    Error::new(format!(
                        "popup.font_size は \"system\" か {} から {} の整数 (ポイント) で書いてください",
                        FONT_SIZE_RANGE.start(),
                        FONT_SIZE_RANGE.end()
                    ))
                })?,
        )
    };
    Ok(Popup {
        scale,
        font,
        font_size,
    })
}

/// 候補の窓と、カーソルのそばの窓の色。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Colors {
    /// 明るい組と暗い組の、どちらを使うか。
    pub theme: ThemeChoice,
    /// 窓の地を透かすときに重ねる地の色の濃さ (百分率)。100 なら透けない。
    pub backdrop_opacity: u8,
    pub light: ColorSet,
    pub dark: ColorSet,
}

/// どちらの組を使うか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    /// アプリの明るさ (Windows の「アプリ モード」) に合わせる。
    Auto,
    Light,
    Dark,
}

/// 一組の色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorSet {
    /// 地。
    pub background: Color,
    /// 文字。
    pub text: Color,
    /// 枠。
    pub border: Color,
    /// 選んでいる行の地 (反転の帯)。
    pub selected_background: Color,
    /// 選んでいる行の文字。
    pub selected_text: Color,
    /// 押すキーを囲む枠 (キーキャップ) と、選んでいる行の印。
    pub key: Color,
}

/// 一つの色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// Windows の標準の色。役目ごとに決まっている。
    System,
    /// Windows のアクセントカラー。
    Accent,
    /// `0xRRGGBB`。
    Rgb(u32),
}

/// 窓の見せ方。エンジンは知らなくてよい値。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// 補完の窓に、変換先と一緒に読みも出すか。
    pub show_reading: bool,
    /// 候補の注釈を窓に出すか。
    pub show_annotation: bool,
}

/// 読めなかった、使えなかった理由。**利用者に見せる文**である。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// 書き足した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filled {
    /// 書き足した後の全文。
    pub text: String,
    /// 書き足した項目。`節.項目` の形。
    pub added: Vec<String>,
    /// ファイルが無く、雛形から作ったか。
    pub created: bool,
}

impl Filled {
    /// ファイルへ書き戻す必要があるか。
    pub fn changed(&self) -> bool {
        self.created || !self.added.is_empty()
    }
}

/// ファイルを読み、足りない項目を書き足し、使える値にして返す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub settings: Settings,
    /// ファイルの全文。書き足した後のもの。
    pub text: String,
    /// 書き足した項目。
    pub added: Vec<String>,
    /// 雛形から作ったか。
    pub created: bool,
    /// 知らない項目。消さずに知らせるだけにする。
    pub unknown: Vec<String>,
    /// ローマ字テーブルのファイル。
    pub romaji_path: PathBuf,
    /// ローマ字テーブルの全文。
    pub romaji_text: String,
    /// ローマ字テーブルが無く、雛形から作ったか。
    pub romaji_created: bool,
}

/// 足りない項目を書き足す。ファイルには触れない。
///
/// `user` が `None` ならファイルが無いということで、雛形そのものを返す。
/// 注記は付けない。**初めから全部が書かれているのだから、書き足した
/// ものは無い。**
pub fn fill(user: Option<&str>) -> Result<Filled, Error> {
    let Some(user) = user else {
        return Ok(Filled {
            text: TEMPLATE.to_owned(),
            added: Vec::new(),
            created: true,
        });
    };

    let template = template();
    let mut doc = parse_document(user)?;
    let mut added = Vec::new();
    fill_table(doc.as_table_mut(), template.as_table(), "", &mut added)?;

    Ok(Filled {
        text: doc.to_string(),
        added,
        created: false,
    })
}

/// `wanted` にあって `present` に無い項目を足す。**入れ子の節にも降りる**
/// (`[colors.light]` の中の一項目が無い、など)。
///
/// 足した項目には注記と雛形の説明を添える。利用者の書いたものには触れない。
fn fill_table(
    present: &mut Table,
    wanted: &Table,
    path: &str,
    added: &mut Vec<String>,
) -> Result<(), Error> {
    for (key, value) in wanted {
        let here = join(path, key);
        match (present.get_mut(key), value) {
            (None, Item::Table(table)) => {
                // 節ごと無い。注記を付けて節ごと足す。
                let mut table = table.clone();
                let prefix = last_paragraph(&decor_text(table.decor().prefix()));
                table
                    .decor_mut()
                    .set_prefix(format!("\n{}{prefix}", added_note()));
                // 雛形での位置を持ち込むと、利用者の節と順番が混ざる。
                table.set_position(None);
                added.extend(leaves(&table, &here));
                present.insert(key, Item::Table(table));
            }
            (None, _) => {
                let (template_key, _) = wanted
                    .get_key_value(key)
                    .expect("いま数え上げた項目なので必ずある");
                let mut new_key = template_key.clone();
                let prefix = decor_text(new_key.leaf_decor().prefix());
                new_key.leaf_decor_mut().set_prefix(format!(
                    "\n{}{}",
                    added_note(),
                    prefix.trim_start()
                ));
                present.insert_formatted(&new_key, value.clone());
                added.push(here);
            }
            (Some(Item::Table(inner)), Item::Table(wanted_inner)) => {
                fill_table(inner, wanted_inner, &here, added)?;
            }
            (Some(_), Item::Table(_)) => {
                return Err(Error::new(format!(
                    "{here} は節として書いてください ([{here}])"
                )));
            }
            // 値が書かれている。正しいかどうかは読むときに確かめる。
            (Some(_), _) => {}
        }
    }
    Ok(())
}

/// 節の前のコメントのうち、節の直前の段落だけ。
///
/// TOML では、ファイルの冒頭の説明は最初の節に付いたコメントとして読まれる。
/// そのまま持ち込むと、**ファイル全体の説明が途中に書き込まれる。** 空行で
/// 区切られた最後の段落だけが、その節の説明である。
fn last_paragraph(prefix: &str) -> String {
    let lines: Vec<&str> = prefix.lines().collect();
    let start = lines
        .iter()
        .rposition(|line| line.trim().is_empty())
        .map_or(0, |at| at + 1);
    lines[start..]
        .iter()
        .map(|line| format!("{line}\n"))
        .collect()
}

/// 節の中の項目を、入れ子まで含めて `節.項目` の形で挙げる。
fn leaves(table: &Table, path: &str) -> Vec<String> {
    table
        .iter()
        .flat_map(|(key, item)| {
            let here = join(path, key);
            match item.as_table() {
                Some(inner) => leaves(inner, &here),
                None => vec![here],
            }
        })
        .collect()
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// 全文を読み、使える値にする。**書き足しはしない。**
///
/// 項目が足りなければ誤りになる。ここに来る全文は、書き足しを済ませた
/// ものでなければならない。ローマ字テーブルは、設定ファイルが指す
/// ファイルの全文を渡す。
pub fn parse(text: &str, romaji: &str) -> Result<Settings, Error> {
    let doc = parse_document(text)?;

    let completion = section(&doc, "completion")?;
    let candidates = section(&doc, "candidates")?;
    let keys = section(&doc, "keys")?;
    let table_name = romaji_table_name(&doc)?;
    let romaji = RomajiTable::parse(romaji)
        .map_err(|e| Error::new(format!("{table_name} を読めません: {e}")))?;

    Ok(Settings {
        engine: Options {
            completion: CompletionOptions {
                dynamic: boolean(completion, "completion", "dynamic")?,
                min_length: count(completion, "completion", "min_length")?,
                limit: count(completion, "completion", "limit")?,
            },
            candidates: CandidateOptions {
                until_list: count(candidates, "candidates", "until_list")?,
                labels: labels(candidates, "candidates", "labels")?,
            },
            romaji,
            keys: keys::keymap(keys)?,
        },
        window: Window {
            show_reading: boolean(completion, "completion", "show_reading")?,
            show_annotation: boolean(candidates, "candidates", "show_annotation")?,
        },
        mode_indicator: {
            let indicator = section(&doc, "mode_indicator")?;
            ModeIndicator {
                on_switch: boolean(indicator, "mode_indicator", "on_switch")?,
                on_focus: boolean(indicator, "mode_indicator", "on_focus")?,
                duration_ms: u32::try_from(count(indicator, "mode_indicator", "duration_ms")?)
                    .map_err(|_| Error::new("mode_indicator.duration_ms が大きすぎます"))?,
            }
        },
        dictionaries: sources(section(&doc, "dictionaries")?)?,
        colors: colors(section(&doc, "colors")?)?,
        popup: popup(section(&doc, "popup")?)?,
        markers: {
            let markers = section(&doc, "markers")?;
            Markers {
                composing: string(markers, "markers", "composing")?,
                composing_empty: string(markers, "markers", "composing_empty")?,
                selecting: string(markers, "markers", "selecting")?,
                okuri: string(markers, "markers", "okuri")?,
            }
        },
        ranker: ranker(section(&doc, "ranker")?)?,
        on_off: keys::on_off(keys)?,
    })
}

/// 並べ替えの節。
fn ranker(table: &Table) -> Result<Ranker, Error> {
    let weight = value(table, "ranker", "weight")?;
    let weight = weight
        .as_float()
        .or_else(|| weight.as_integer().map(|n| n as f64))
        .filter(|w| w.is_finite() && *w >= 0.0)
        .ok_or_else(|| Error::new("ranker.weight は 0 以上の数で書いてください"))?;
    Ok(Ranker {
        enabled: boolean(table, "ranker", "enabled")?,
        weight: Weight(weight as f32),
        deadline_ms: u32::try_from(count(table, "ranker", "deadline_ms")?)
            .map_err(|_| Error::new("ranker.deadline_ms が大きすぎます"))?,
        before: count(table, "ranker", "before")?,
        after: amount(table, "ranker", "after")?,
        top: count(table, "ranker", "top")?,
        min_context: amount(table, "ranker", "min_context")?,
        threads: count(table, "ranker", "threads")?,
    })
}

/// 雛形に無い項目を挙げる。
pub fn unknown_keys(text: &str) -> Vec<String> {
    let Ok(doc) = parse_document(text) else {
        return Vec::new();
    };
    let template = template();
    let mut unknown = Vec::new();
    unknown_in(doc.as_table(), template.as_table(), "", &mut unknown);
    unknown
}

fn unknown_in(present: &Table, known: &Table, path: &str, out: &mut Vec<String>) {
    for (key, item) in present {
        let here = join(path, key);
        match (item.as_table(), known.get(key)) {
            (_, None) => out.push(here),
            (Some(inner), Some(Item::Table(known_inner))) => {
                unknown_in(inner, known_inner, &here, out);
            }
            _ => {}
        }
    }
}

/// ファイルを読み、足りない項目を書き足してから使える値にする。
///
/// ファイルが無ければ雛形から作る。書き足したときだけ書き戻す。**書き
/// 戻すのは一つのプロセスだけにする** — 同じファイルに二人が書き足すと、
/// 同じ項目が二度書かれて読めなくなる。
pub fn load(path: &Path) -> Result<Loaded, Error> {
    let existing = match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(Error::new(format!(
                "設定ファイルを読めません ({}): {e}",
                path.display()
            )));
        }
    };

    // 改行は LF に揃えて扱い、書き戻すときに元の流儀に戻す。メモ帳で
    // 書いたファイルは CRLF で、先頭に BOM が付いていることもある。
    let crlf = existing.as_deref().is_some_and(|t| t.contains("\r\n"));
    let normalized = existing.map(|t| t.trim_start_matches('\u{feff}').replace("\r\n", "\n"));

    let filled = fill(normalized.as_deref())?;
    if filled.changed() {
        let text = if crlf {
            filled.text.replace('\n', "\r\n")
        } else {
            filled.text.clone()
        };
        write(path, &text).map_err(|e| {
            Error::new(format!(
                "設定ファイルを書けません ({}): {e}",
                path.display()
            ))
        })?;
    }

    // ローマ字テーブル。**無いときだけ作る。あれば触らない。**
    let romaji_path = romaji_path(path, &filled.text)?;
    let (romaji_text, romaji_created) = match fs::read_to_string(&romaji_path) {
        Ok(text) => (text.trim_start_matches('\u{feff}').to_owned(), false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            write(&romaji_path, ROMAJI_TEMPLATE).map_err(|e| {
                Error::new(format!(
                    "ローマ字テーブルを作れません ({}): {e}",
                    romaji_path.display()
                ))
            })?;
            (ROMAJI_TEMPLATE.to_owned(), true)
        }
        Err(e) => {
            return Err(Error::new(format!(
                "ローマ字テーブルを読めません ({}): {e}",
                romaji_path.display()
            )));
        }
    };

    let settings = parse(&filled.text, &romaji_text)?;
    let unknown = unknown_keys(&filled.text);
    Ok(Loaded {
        settings,
        text: filled.text,
        added: filled.added,
        created: filled.created,
        unknown,
        romaji_path,
        romaji_text,
        romaji_created,
    })
}

/// 設定ファイルを雛形で上書きする。元の中身は隣に退避する。
///
/// 返すのは退避先。元のファイルが無ければ `None`。
pub fn reset_settings(path: &Path) -> Result<Option<PathBuf>, Error> {
    overwrite_with(path, TEMPLATE)
}

/// ローマ字テーブルを雛形で上書きする。元の中身は隣に退避する。
///
/// どのファイルかは設定ファイルが決める。返すのは上書きしたファイルと
/// 退避先。
pub fn reset_romaji(settings: &Path) -> Result<(PathBuf, Option<PathBuf>), Error> {
    let text = fs::read_to_string(settings).map_err(|e| {
        Error::new(format!(
            "設定ファイルを読めないので、ローマ字テーブルの場所が分かりません: {e}"
        ))
    })?;
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let target = romaji_path(settings, &text)?;
    let backup = overwrite_with(&target, ROMAJI_TEMPLATE)?;
    Ok((target, backup))
}

/// 雛形で上書きする。**元の中身は消さずに退避する。**
///
/// 退避先は `<名前>.bak`、それがあれば `<名前>.bak2`、`.bak3` と空いている
/// ものを選ぶ。前の退避を上書きしない。
fn overwrite_with(path: &Path, template: &str) -> Result<Option<PathBuf>, Error> {
    let backup = if path.exists() {
        let backup = free_backup_name(path);
        fs::copy(path, &backup).map_err(|e| {
            Error::new(format!(
                "退避できないので、上書きをやめました ({}): {e}",
                backup.display()
            ))
        })?;
        Some(backup)
    } else {
        None
    };
    write(path, template)
        .map_err(|e| Error::new(format!("書けません ({}): {e}", path.display())))?;
    Ok(backup)
}

fn free_backup_name(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    (1..)
        .map(|n| {
            let suffix = if n == 1 { String::new() } else { n.to_string() };
            path.with_file_name(format!("{name}.bak{suffix}"))
        })
        .find(|candidate| !candidate.exists())
        .expect("空いている名前はいずれ見つかる")
}

/// ローマ字テーブルの場所。設定ファイルと同じ場所からの相対で解く。
fn romaji_path(settings: &Path, text: &str) -> Result<PathBuf, Error> {
    let doc = parse_document(text)?;
    let name = romaji_table_name(&doc)?;
    let name = Path::new(&name);
    if name.is_absolute() {
        return Ok(name.to_path_buf());
    }
    Ok(match settings.parent() {
        Some(directory) => directory.join(name),
        None => name.to_path_buf(),
    })
}

fn romaji_table_name(doc: &DocumentMut) -> Result<String, Error> {
    let romaji = section(doc, "romaji")?;
    value(romaji, "romaji", "table")?
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::new("romaji.table にはファイルの名前を書いてください (例: \"romaji.txt\")")
        })
}

/// 書きかけで途切れても元のファイルを壊さないよう、隣に書いてから置き換える。
fn write(path: &Path, text: &str) -> io::Result<()> {
    // 相対の名前だけなら親は空になる。作るものは無い。
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".writing");
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path)
}

fn template() -> DocumentMut {
    TEMPLATE
        .parse()
        .expect("同梱の雛形は必ず読める (試験で確かめている)")
}

fn parse_document(text: &str) -> Result<DocumentMut, Error> {
    text.parse::<DocumentMut>()
        .map_err(|e| Error::new(format!("設定ファイルを読めません: {e}")))
}

fn decor_text(raw: Option<&toml_edit::RawString>) -> String {
    raw.and_then(toml_edit::RawString::as_str)
        .unwrap_or("")
        .to_owned()
}

fn section<'a>(doc: &'a DocumentMut, name: &str) -> Result<&'a Table, Error> {
    match doc.get(name) {
        Some(Item::Table(table)) => Ok(table),
        Some(_) => Err(Error::new(format!(
            "{name} は節として書いてください ([{name}])"
        ))),
        None => Err(Error::new(format!("[{name}] がありません"))),
    }
}

pub(crate) fn value<'a>(table: &'a Table, section: &str, key: &str) -> Result<&'a Item, Error> {
    table
        .get(key)
        .ok_or_else(|| Error::new(format!("{section}.{key} がありません")))
}

fn boolean(table: &Table, section: &str, key: &str) -> Result<bool, Error> {
    value(table, section, key)?
        .as_bool()
        .ok_or_else(|| Error::new(format!("{section}.{key} は true か false で書いてください")))
}

/// 1 以上の整数。
fn count(table: &Table, section: &str, key: &str) -> Result<usize, Error> {
    value(table, section, key)?
        .as_integer()
        .filter(|n| *n >= 1)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| Error::new(format!("{section}.{key} は 1 以上の整数で書いてください")))
}

/// 0 以上の整数。
fn amount(table: &Table, section: &str, key: &str) -> Result<usize, Error> {
    value(table, section, key)?
        .as_integer()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| Error::new(format!("{section}.{key} は 0 以上の整数で書いてください")))
}

/// 文字列。空でもよい。
fn string(table: &Table, section: &str, key: &str) -> Result<String, Error> {
    value(table, section, key)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::new(format!(
                "{section}.{key} は文字列で書いてください (例: \"▽\")"
            ))
        })
}

/// 色の節。
fn colors(table: &Table) -> Result<Colors, Error> {
    let theme = match value(table, "colors", "theme")?.as_str() {
        Some("auto") => ThemeChoice::Auto,
        Some("light") => ThemeChoice::Light,
        Some("dark") => ThemeChoice::Dark,
        _ => {
            return Err(Error::new(
                "colors.theme は \"auto\" か \"light\" か \"dark\" で書いてください",
            ));
        }
    };
    let backdrop_opacity = value(table, "colors", "backdrop_opacity")?
        .as_integer()
        .and_then(|n| u8::try_from(n).ok())
        .filter(|n| *n <= 100)
        .ok_or_else(|| {
            Error::new("colors.backdrop_opacity は 0 から 100 の整数で書いてください")
        })?;
    Ok(Colors {
        theme,
        backdrop_opacity,
        light: color_set(table, "light")?,
        dark: color_set(table, "dark")?,
    })
}

fn color_set(colors: &Table, name: &str) -> Result<ColorSet, Error> {
    let path = format!("colors.{name}");
    let table = match colors.get(name) {
        Some(Item::Table(table)) => table,
        Some(_) => {
            return Err(Error::new(format!(
                "{path} は節として書いてください ([{path}])"
            )));
        }
        None => return Err(Error::new(format!("[{path}] がありません"))),
    };
    Ok(ColorSet {
        background: color(table, &path, "background")?,
        text: color(table, &path, "text")?,
        border: color(table, &path, "border")?,
        selected_background: color(table, &path, "selected_background")?,
        selected_text: color(table, &path, "selected_text")?,
        key: color(table, &path, "key")?,
    })
}

/// `"#RRGGBB"`、`"system"`、`"accent"` のどれか。
fn color(table: &Table, section: &str, key: &str) -> Result<Color, Error> {
    let wrong = || {
        Error::new(format!(
            "{section}.{key} は \"#RRGGBB\" か \"system\" か \"accent\" で書いてください"
        ))
    };
    let text = value(table, section, key)?.as_str().ok_or_else(wrong)?;
    match text {
        "system" => Ok(Color::System),
        "accent" => Ok(Color::Accent),
        _ => {
            let hex = text
                .strip_prefix('#')
                .filter(|h| h.len() == 6)
                .ok_or_else(wrong)?;
            u32::from_str_radix(hex, 16)
                .map(Color::Rgb)
                .map_err(|_| wrong())
        }
    }
}

/// 辞書の並び。空でもよい (ユーザー辞書だけで使う)。
///
/// 一つひとつは在りかの文字列か、`{ katakana_from = "在りか" }`。
fn sources(table: &Table) -> Result<Vec<Source>, Error> {
    let wrong = || {
        Error::new(
            "dictionaries.sources は在りかの並びで書いてください \
             (例: [\"https://…/SKK-JISYO.L\"])",
        )
    };
    let place = |text: &str| {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::new("dictionaries.sources に空の在りかがあります"));
        }
        Ok(Source::parse(text))
    };
    let array = value(table, "dictionaries", "sources")?
        .as_array()
        .ok_or_else(wrong)?;
    array
        .iter()
        .map(|item| {
            if let Some(text) = item.as_str() {
                return place(text);
            }
            let Some(derived) = item.as_inline_table() else {
                return Err(wrong());
            };
            // 知らない書き方は、推し量らずに断る。
            let keys: Vec<&str> = derived.iter().map(|(key, _)| key).collect();
            if keys != ["katakana_from"] {
                return Err(Error::new(format!(
                    "dictionaries.sources の {{ {} }} は分かりません \
                     (書けるのは {{ katakana_from = \"在りか\" }})",
                    keys.join(", ")
                )));
            }
            let from = derived
                .get("katakana_from")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    Error::new("dictionaries.sources の katakana_from には在りかを書いてください")
                })?;
            Ok(Source::Katakana(Box::new(place(from)?)))
        })
        .collect()
}

/// 候補を選ぶキーの並び。
fn labels(table: &Table, section: &str, key: &str) -> Result<Vec<char>, Error> {
    let text = value(table, section, key)?.as_str().ok_or_else(|| {
        Error::new(format!(
            "{section}.{key} は文字列で書いてください (例: \"asdfjkl\")"
        ))
    })?;
    let labels: Vec<char> = text.chars().collect();
    if labels.is_empty() {
        return Err(Error::new(format!(
            "{section}.{key} には一文字以上書いてください"
        )));
    }
    if labels.iter().any(|c| c.is_whitespace()) {
        return Err(Error::new(format!("{section}.{key} に空白は使えません")));
    }
    for (at, c) in labels.iter().enumerate() {
        if labels[..at].contains(c) {
            // 同じキーが二つあると、後ろの候補を選べなくなる。
            return Err(Error::new(format!(
                "{section}.{key} に「{c}」が二度出てきます"
            )));
        }
    }
    Ok(labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 雛形から一行を抜いたもの。
    fn without(line: &str) -> String {
        assert!(TEMPLATE.contains(line), "雛形に {line:?} が無い");
        TEMPLATE.replace(&format!("{line}\n"), "")
    }

    #[test]
    fn the_template_holds_every_setting() {
        // **雛形に書き忘れた項目は、どこからも値が来ない。** 項目を足して
        // 雛形に書き忘れたら、ここで落ちる。
        let settings = parse(TEMPLATE, ROMAJI_TEMPLATE).expect("雛形はそのまま使える");
        assert!(settings.engine.completion.dynamic);
    }

    #[test]
    fn the_template_shows_a_space_only_when_the_reading_is_empty() {
        // SKKFEP や CorvusSKK と同じく、読みが無くなって初めて空白を出す。
        // 普通の空白ではなく U+00A0 にする。U+0020 は下線を省くアプリがある。
        let settings = parse(TEMPLATE, ROMAJI_TEMPLATE).expect("雛形はそのまま使える");
        assert_eq!(settings.markers.composing, "");
        assert_eq!(settings.markers.composing_empty, "\u{00A0}");
        assert_eq!(settings.markers.selecting, "");
        assert_eq!(settings.markers.okuri, "");
    }

    #[test]
    fn the_template_knows_no_strangers() {
        assert!(unknown_keys(TEMPLATE).is_empty());
    }

    #[test]
    fn a_missing_file_becomes_the_template() {
        let filled = fill(None).unwrap();
        assert!(filled.created);
        assert!(filled.added.is_empty(), "初めから全部書かれている");
        assert_eq!(filled.text, TEMPLATE);
    }

    #[test]
    fn a_complete_file_is_left_alone() {
        let filled = fill(Some(TEMPLATE)).unwrap();
        assert!(!filled.changed());
        assert_eq!(filled.text, TEMPLATE);
    }

    #[test]
    fn a_missing_setting_is_written_in_with_a_note() {
        let user = without("limit = 16");
        let filled = fill(Some(&user)).unwrap();

        assert_eq!(filled.added, ["completion.limit"]);
        assert!(filled.text.contains("limit = 16"));
        assert!(
            filled.text.contains("で書き足しました"),
            "書き足したことが見て分かる"
        );
        // 雛形の説明も一緒に入る。**何の項目か分からないものを足さない。**
        assert!(filled.text.contains("# 一度に覚えておく補完の数"));
        assert!(parse(&filled.text, ROMAJI_TEMPLATE).is_ok());
    }

    #[test]
    fn a_setting_is_written_into_its_own_section() {
        // 末尾に足すと、別の節の項目になってしまう。
        let user = without("limit = 16");
        let filled = fill(Some(&user)).unwrap();
        let doc: DocumentMut = filled.text.parse().unwrap();
        assert!(doc["completion"].as_table().unwrap().contains_key("limit"));
        assert!(!doc["candidates"].as_table().unwrap().contains_key("limit"));
    }

    #[test]
    fn a_missing_section_is_written_in_whole() {
        let user =
            "[completion]\ndynamic = false\nmin_length = 3\nlimit = 8\nshow_reading = true\n";
        let filled = fill(Some(user)).unwrap();
        // [completion] 以外の節の項目が、すべて書き足される。
        let expected: Vec<String> = leaves(template().as_table(), "")
            .into_iter()
            .filter(|key| !key.starts_with("completion."))
            .collect();
        assert_eq!(filled.added, expected);
        assert!(filled.added.contains(&"candidates.labels".to_owned()));
        assert!(filled.added.contains(&"colors.dark.background".to_owned()));
        let settings = parse(&filled.text, ROMAJI_TEMPLATE).unwrap();
        assert_eq!(
            settings.engine.candidates.labels,
            "asdfjkl".chars().collect::<Vec<_>>()
        );
    }

    #[test]
    fn what_the_user_wrote_is_kept_as_it_was() {
        // 利用者のコメントと値は、一文字も崩さない。
        let user =
            without("limit = 16").replace("min_length = 2", "min_length = 3  # 二文字だと多すぎた");
        let filled = fill(Some(&user)).unwrap();
        assert!(filled.text.contains("min_length = 3  # 二文字だと多すぎた"));
        assert!(
            filled
                .text
                .starts_with(&user[..user.find("[completion]").unwrap()])
        );
    }

    #[test]
    fn the_file_wins_over_the_template() {
        let user = TEMPLATE.replace("min_length = 2", "min_length = 3");
        let settings = parse(&user, ROMAJI_TEMPLATE).unwrap();
        assert_eq!(settings.engine.completion.min_length, 3);
    }

    #[test]
    fn nothing_is_taken_from_the_template_when_parsing() {
        // **読むときに雛形を見ない。** 足りなければ誤りになる。
        let user = without("limit = 16");
        let error = parse(&user, ROMAJI_TEMPLATE).unwrap_err();
        assert!(error.to_string().contains("completion.limit"));
    }

    #[test]
    fn strangers_are_reported_not_removed() {
        let user = TEMPLATE.replace("limit = 16", "limit = 16\nlimt = 3") + "\n[mystery]\nx = 1\n";
        let filled = fill(Some(&user)).unwrap();
        assert!(filled.text.contains("limt = 3"), "消さない");
        assert_eq!(unknown_keys(&filled.text), ["completion.limt", "mystery"]);
    }

    #[test]
    fn the_ranker_is_on_in_the_template() {
        let ranker = parse(TEMPLATE, ROMAJI_TEMPLATE).unwrap().ranker;
        assert!(ranker.enabled);
        assert_eq!(ranker.weight, Weight(1.0));
        assert_eq!((ranker.before, ranker.after), (100, 5));
    }

    #[test]
    fn a_whole_number_is_a_weight_too() {
        let user = TEMPLATE.replace("weight = 1.0", "weight = 2");
        let settings = parse(&user, ROMAJI_TEMPLATE).unwrap();
        assert_eq!(settings.ranker.weight, Weight(2.0));
    }

    #[test]
    fn broken_values_are_explained() {
        let cases = [
            ("labels = \"asdfjkl\"", "labels = \"\"", "candidates.labels"),
            ("labels = \"asdfjkl\"", "labels = \"asdfa\"", "二度"),
            ("abbrev = [\"/\"]", "abbrev = \"/\"", "配列"),
            ("abbrev = [\"/\"]", "abbrev = [\"Alt+/\"]", "使えないキー"),
            (
                "abbrev = [\"/\"]",
                "abbrev = [\"q\"]",
                "keys.toggle_kana と keys.abbrev",
            ),
            (
                "on_off = [\"半角/全角\", \"Alt+`\"]",
                "on_off = [\"a\"]",
                "keys.on_off",
            ),
            ("min_length = 2", "min_length = 0", "1 以上"),
            ("dynamic = true", "dynamic = \"yes\"", "true か false"),
            ("until_list = 5", "until_list = 2.5", "整数"),
            ("weight = 1.0", "weight = -1.0", "ranker.weight"),
            ("after = 5", "after = -1", "0 以上の整数"),
            ("threads = 4", "threads = \"four\"", "ranker.threads"),
            ("top = 7", "top = 0", "ranker.top"),
            ("min_context = 2", "min_context = -1", "ranker.min_context"),
        ];
        for (from, to, expected) in cases {
            let user = TEMPLATE.replace(from, to);
            let error = parse(&user, ROMAJI_TEMPLATE).expect_err(to);
            assert!(error.to_string().contains(expected), "{to}: {error}");
        }
    }

    #[test]
    fn the_template_keeps_the_familiar_keys() {
        use crystalskk_core::{Command, Key};
        let keys = parse(TEMPLATE, ROMAJI_TEMPLATE).unwrap().engine.keys;
        assert_eq!(keys.command(Key::Ctrl('j')), Some(Command::Hiragana));
        assert_eq!(keys.command(Key::Enter), Some(Command::Kakutei));
        assert_eq!(keys.command(Key::Escape), Some(Command::Cancel));
        assert_eq!(keys.command(Key::Space), Some(Command::StartHenkan));
        assert_eq!(keys.first_char(Command::TakeCompletion), Some('.'));
    }

    #[test]
    fn a_command_can_be_left_without_keys() {
        use crystalskk_core::Command;
        let user = TEMPLATE.replace("purge = [\"X\"]", "purge = []");
        let keys = parse(&user, ROMAJI_TEMPLATE).unwrap().engine.keys;
        assert_eq!(keys.keys(Command::Purge).count(), 0);
    }

    #[test]
    fn something_must_turn_the_input_on() {
        let user = TEMPLATE
            .replace("on_off = [\"半角/全角\", \"Alt+`\"]", "on_off = []")
            .replace("on = [\"ImeOn\"]", "on = []");
        let error = parse(&user, ROMAJI_TEMPLATE).unwrap_err();
        assert!(error.to_string().contains("入にするキー"), "{error}");
    }

    #[test]
    fn a_section_written_as_a_value_is_refused() {
        let error = fill(Some("completion = 1\n")).unwrap_err();
        assert!(error.to_string().contains("[completion]"));
    }

    #[test]
    fn the_template_lists_the_l_dictionary_and_its_katakana_words() {
        let settings = parse(TEMPLATE, ROMAJI_TEMPLATE).unwrap();
        let l = Source::Url(
            "https://raw.githubusercontent.com/skk-dev/dict/master/SKK-JISYO.L".to_owned(),
        );
        assert_eq!(
            settings.dictionaries,
            [l.clone(), Source::Katakana(Box::new(l))]
        );
    }

    #[test]
    fn a_derived_dictionary_names_where_it_comes_from() {
        let source = Source::Katakana(Box::new(Source::File("my.dict".to_owned())));
        assert_eq!(source.base(), &Source::File("my.dict".to_owned()));
        assert!(source.is_katakana());
        assert_eq!(
            source.resolve(Path::new("D:/settings")),
            Some(Path::new("D:/settings").join("my.dict"))
        );
        assert_eq!(source.as_written(), "{ katakana_from = \"my.dict\" }");
    }

    #[test]
    fn an_unknown_way_of_deriving_is_refused() {
        let with = |entry: &str| {
            let start = TEMPLATE.find("sources = [").unwrap();
            let end = TEMPLATE[start..].find("\n]").unwrap() + start + 2;
            format!(
                "{}sources = [{entry}]{}",
                &TEMPLATE[..start],
                &TEMPLATE[end..]
            )
        };
        for broken in [
            "{ hiragana_from = \"a\" }",
            "{ katakana_from = 1 }",
            "{ katakana_from = \"a\", extra = 1 }",
        ] {
            let error = parse(&with(broken), ROMAJI_TEMPLATE).expect_err(broken);
            assert!(
                error.to_string().contains("dictionaries.sources"),
                "{error}"
            );
        }
        assert!(parse(&with("{ katakana_from = \"a.dict\" }"), ROMAJI_TEMPLATE).is_ok());
    }

    #[test]
    fn urls_and_files_are_told_apart() {
        let start = TEMPLATE.find("sources = [").unwrap();
        let end = TEMPLATE[start..].find("\n]").unwrap() + start + 2;
        let user = format!(
            "{}sources = [\"HTTPS://example.com/a\", \"my.dict\", \"C:/dicts/b.dict\"]{}",
            &TEMPLATE[..start],
            &TEMPLATE[end..]
        );
        let settings = parse(&user, ROMAJI_TEMPLATE).unwrap();
        assert_eq!(
            settings.dictionaries,
            [
                Source::Url("HTTPS://example.com/a".to_owned()),
                Source::File("my.dict".to_owned()),
                Source::File("C:/dicts/b.dict".to_owned()),
            ]
        );
        let here = Path::new("D:/settings");
        assert_eq!(
            settings.dictionaries[1].resolve(here),
            Some(here.join("my.dict"))
        );
        assert_eq!(settings.dictionaries[0].resolve(here), None);
    }

    #[test]
    fn no_dictionaries_is_a_choice() {
        // ユーザー辞書だけで使うこともできる。
        let start = TEMPLATE.find("sources = [").unwrap();
        let end = TEMPLATE[start..].find("\n]").unwrap() + start + 2;
        let user = format!("{}sources = []{}", &TEMPLATE[..start], &TEMPLATE[end..]);
        assert!(
            parse(&user, ROMAJI_TEMPLATE)
                .unwrap()
                .dictionaries
                .is_empty()
        );
    }

    #[test]
    fn a_broken_dictionary_list_is_explained() {
        for broken in [
            "sources = \"one.dict\"",
            "sources = [1]",
            "sources = [\"\"]",
        ] {
            let start = TEMPLATE.find("sources = [").unwrap();
            let end = TEMPLATE[start..].find("\n]").unwrap() + start + 2;
            let user = format!("{}{broken}{}", &TEMPLATE[..start], &TEMPLATE[end..]);
            let error = parse(&user, ROMAJI_TEMPLATE).expect_err(broken);
            assert!(
                error.to_string().contains("dictionaries.sources"),
                "{error}"
            );
        }
    }

    #[test]
    fn the_mode_indicator_shows_on_switch_for_a_second() {
        let indicator = parse(TEMPLATE, ROMAJI_TEMPLATE).unwrap().mode_indicator;
        assert_eq!(
            indicator,
            ModeIndicator {
                on_switch: true,
                on_focus: false,
                duration_ms: 1000,
            }
        );
    }

    #[test]
    fn the_template_grounds_are_close_to_windows_11_menus() {
        // 標準の地の色 (`COLOR_WINDOW`) は真っ白で、ダークモードにも従わない。
        let colors = parse(TEMPLATE, ROMAJI_TEMPLATE).unwrap().colors;
        assert_eq!(colors.theme, ThemeChoice::Auto);
        assert_eq!(colors.light.background, Color::Rgb(0xF9_F9_F9));
        assert_eq!(colors.light.text, Color::Rgb(0x00_00_00));
        assert_eq!(colors.dark.background, Color::Rgb(0x2B_2B_2B));
        assert_eq!(colors.dark.text, Color::Rgb(0xFF_FF_FF));
    }

    #[test]
    fn the_popups_follow_windows_until_told_otherwise() {
        let popup = parse(TEMPLATE, ROMAJI_TEMPLATE).unwrap().popup;
        assert_eq!(
            popup,
            Popup {
                scale: 100,
                font: None,
                font_size: None,
            }
        );
    }

    #[test]
    fn the_popups_can_be_resized_and_given_a_font() {
        let user = TEMPLATE
            .replacen("scale = 100", "scale = 125", 1)
            .replacen("font = \"system\"", "font = \"BIZ UDPGothic\"", 1)
            .replacen("font_size = \"system\"", "font_size = 11", 1);
        let popup = parse(&user, ROMAJI_TEMPLATE).unwrap().popup;
        assert_eq!(popup.scale, 125);
        assert_eq!(popup.font.as_deref(), Some("BIZ UDPGothic"));
        assert_eq!(popup.font_size, Some(11));
    }

    #[test]
    fn popup_values_out_of_range_are_told() {
        for (from, to) in [
            ("scale = 100", "scale = 10"),
            ("font = \"system\"", "font = \"\""),
            ("font_size = \"system\"", "font_size = 200"),
        ] {
            let user = TEMPLATE.replacen(from, to, 1);
            assert!(parse(&user, ROMAJI_TEMPLATE).is_err(), "{to}");
        }
    }

    #[test]
    fn colours_are_read_in_three_ways() {
        let user = TEMPLATE
            .replacen("border = \"system\"", "border = \"accent\"", 1)
            .replacen("text = \"#000000\"", "text = \"#1a2B3c\"", 1);
        let light = parse(&user, ROMAJI_TEMPLATE).unwrap().colors.light;
        assert_eq!(light.border, Color::Accent);
        assert_eq!(light.text, Color::Rgb(0x1A_2B_3C), "大文字小文字は問わない");
    }

    #[test]
    fn a_broken_colour_is_explained() {
        for broken in ["\"#12345\"", "\"blue\"", "\"#GGGGGG\"", "1"] {
            let user = TEMPLATE.replacen("border = \"system\"", &format!("border = {broken}"), 1);
            let error = parse(&user, ROMAJI_TEMPLATE).expect_err(broken);
            assert!(error.to_string().contains("colors.light.border"), "{error}");
        }
        let user = TEMPLATE.replace("theme = \"auto\"", "theme = \"sepia\"");
        assert!(parse(&user, ROMAJI_TEMPLATE).is_err());
    }

    #[test]
    fn a_missing_setting_deep_inside_is_written_into_its_own_table() {
        // 入れ子の節の中の一項目。**末尾に足すと別の節の項目になる。**
        let user = TEMPLATE.replacen("selected_text = \"system\"\n", "", 1);
        let filled = fill(Some(&user)).unwrap();
        assert_eq!(filled.added, ["colors.light.selected_text"]);
        let doc: DocumentMut = filled.text.parse().unwrap();
        assert!(
            doc["colors"]["light"]
                .as_table()
                .unwrap()
                .contains_key("selected_text")
        );
        assert!(parse(&filled.text, ROMAJI_TEMPLATE).is_ok());
    }

    #[test]
    fn a_missing_table_deep_inside_is_written_in_whole() {
        // 説明文にも `[colors.dark]` という文字があるので、行の頭で探す。
        let start = TEMPLATE.find("\n[colors.dark]").unwrap() + 1;
        let end = TEMPLATE[start..]
            .find("\n\n")
            .map_or(TEMPLATE.len(), |e| start + e + 1);
        let user = format!("{}{}", &TEMPLATE[..start], &TEMPLATE[end..]);
        let filled = fill(Some(&user)).unwrap();
        assert!(
            filled.added.contains(&"colors.dark.background".to_owned()),
            "{:?}",
            filled.added
        );
        assert!(parse(&filled.text, ROMAJI_TEMPLATE).is_ok());
    }

    #[test]
    fn the_file_header_is_not_copied_into_the_middle() {
        // [completion] ごと無いファイル。雛形の冒頭の説明は [completion] に
        // 付いたコメントとして読まれるが、途中に書き込んではいけない。
        let user = "[candidates]\nuntil_list = 5\nlabels = \"asdfjkl\"\n";
        let filled = fill(Some(user)).unwrap();
        assert!(
            !filled.text.contains("# CrystalSKK の設定"),
            "{}",
            filled.text
        );
        assert!(
            filled.text.contains("# 打っている最中に"),
            "項目の説明は持ち込む"
        );
        // 節の直前の説明は持ち込む。
        let start = TEMPLATE.find("\n[colors.dark]").unwrap() + 1;
        let end = TEMPLATE[start..]
            .find("\n\n")
            .map_or(TEMPLATE.len(), |e| start + e + 1);
        let without_dark = format!("{}{}", &TEMPLATE[..start], &TEMPLATE[end..]);
        let filled = fill(Some(&without_dark)).unwrap();
        assert!(filled.text.contains("# 暗い組。"), "{}", filled.text);
    }

    #[test]
    fn strangers_deep_inside_are_reported() {
        let user = TEMPLATE.replacen(
            "border = \"system\"",
            "border = \"system\"\nshadow = \"#000000\"",
            1,
        );
        assert_eq!(unknown_keys(&user), ["colors.light.shadow"]);
    }

    #[test]
    fn a_broken_romaji_table_is_named_with_its_line() {
        let error = parse(TEMPLATE, "ka\tか\nki き\n").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("romaji.txt"), "{message}");
        assert!(message.contains("2 行目"), "{message}");
    }

    /// 試験ごとに空の置き場所を作る。
    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "crystalskk-settings-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn the_romaji_table_is_made_once_and_then_left_alone() {
        let directory = scratch("romaji");
        let path = directory.join("config.toml");
        let table = directory.join("romaji.txt");

        let first = load(&path).unwrap();
        assert!(first.romaji_created);
        assert_eq!(fs::read_to_string(&table).unwrap(), ROMAJI_TEMPLATE);

        // 利用者が規則を一つだけにしたとする。**書き足さない。**
        fs::write(&table, "ka\tか\n").unwrap();
        let second = load(&path).unwrap();
        assert!(!second.romaji_created);
        assert_eq!(fs::read_to_string(&table).unwrap(), "ka\tか\n");
        assert_eq!(second.romaji_text, "ka\tか\n");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_table_can_live_elsewhere() {
        let directory = scratch("elsewhere");
        let path = directory.join("config.toml");
        fs::write(
            &path,
            TEMPLATE.replace("table = \"romaji.txt\"", "table = \"azik.txt\""),
        )
        .unwrap();
        fs::write(directory.join("azik.txt"), "ka\tか\n").unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.romaji_path, directory.join("azik.txt"));
        assert!(!directory.join("romaji.txt").exists());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn resetting_keeps_what_was_there() {
        let directory = scratch("reset");
        let path = directory.join("config.toml");
        load(&path).unwrap();

        let mine = TEMPLATE.replace("min_length = 2", "min_length = 3");
        fs::write(&path, &mine).unwrap();
        let backup = reset_settings(&path).unwrap().expect("退避した");
        assert_eq!(fs::read_to_string(&path).unwrap(), TEMPLATE);
        assert_eq!(fs::read_to_string(&backup).unwrap(), mine, "元の中身が残る");

        // 二度目は別の名前に退避する。**前の退避を消さない。**
        fs::write(&path, "# 二度目\n").unwrap();
        let second = reset_settings(&path).unwrap().expect("退避した");
        assert_ne!(second, backup);
        assert_eq!(fs::read_to_string(&backup).unwrap(), mine);

        let table = directory.join("romaji.txt");
        fs::write(&table, "ka\tか\n").unwrap();
        let (target, saved) = reset_romaji(&path).unwrap();
        assert_eq!(target, table);
        assert_eq!(fs::read_to_string(&table).unwrap(), ROMAJI_TEMPLATE);
        assert_eq!(fs::read_to_string(saved.unwrap()).unwrap(), "ka\tか\n");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn loading_creates_then_fills_the_file() {
        let directory =
            std::env::temp_dir().join(format!("crystalskk-settings-test-{}", std::process::id()));
        let path = directory.join("config.toml");
        let _ = fs::remove_dir_all(&directory);

        let first = load(&path).unwrap();
        assert!(first.created);
        assert_eq!(fs::read_to_string(&path).unwrap(), TEMPLATE);

        // 利用者が一つ消し、CRLF で保存したとする。
        let edited = without("limit = 16").replace('\n', "\r\n");
        fs::write(&path, &edited).unwrap();
        let second = load(&path).unwrap();
        assert_eq!(second.added, ["completion.limit"]);
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("limit = 16\r\n"), "改行の流儀を保つ");
        assert!(
            !written.replace("\r\n", "").contains('\n'),
            "LF が混ざらない"
        );

        let third = load(&path).unwrap();
        assert!(third.added.is_empty(), "一度書き足せば、もう足さない");
        let _ = fs::remove_dir_all(&directory);
    }
}
