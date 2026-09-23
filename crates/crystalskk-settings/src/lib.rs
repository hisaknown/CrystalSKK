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

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use crystalskk_core::options::{CandidateOptions, CompletionOptions, Options};
use toml_edit::{DocumentMut, Item, Table};

/// 雛形。ファイルを作るときと、足りない項目を書き足すときだけに使う。
pub const TEMPLATE: &str = include_str!("../default.toml");

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
}

/// 窓の見せ方。エンジンは知らなくてよい値。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// 補完の窓に、変換先と一緒に読みも出すか。
    pub show_reading: bool,
}

/// 読めなかった、使えなかった理由。**利用者に見せる文**である。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    fn new(message: impl Into<String>) -> Self {
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

    for (section, item) in template.as_table() {
        let Some(wanted) = item.as_table() else {
            continue;
        };
        match doc.get_mut(section) {
            None => {
                // 節ごと無い。注記を付けて節ごと足す。
                let mut table = wanted.clone();
                let prefix = decor_text(table.decor().prefix());
                table
                    .decor_mut()
                    .set_prefix(format!("\n{}{}", added_note(), prefix.trim_start()));
                // 雛形での位置を持ち込むと、利用者の節と順番が混ざる。
                table.set_position(None);
                doc.insert(section, Item::Table(table));
                added.extend(wanted.iter().map(|(key, _)| format!("{section}.{key}")));
            }
            Some(Item::Table(present)) => {
                for (key, value) in wanted {
                    if present.contains_key(key) {
                        continue;
                    }
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
                    added.push(format!("{section}.{key}"));
                }
            }
            Some(_) => {
                return Err(Error::new(format!(
                    "{section} は節として書いてください ([{section}])"
                )));
            }
        }
    }

    Ok(Filled {
        text: doc.to_string(),
        added,
        created: false,
    })
}

/// 全文を読み、使える値にする。**書き足しはしない。**
///
/// 項目が足りなければ誤りになる。ここに来る全文は、書き足しを済ませた
/// ものでなければならない。
pub fn parse(text: &str) -> Result<Settings, Error> {
    let doc = parse_document(text)?;

    let completion = section(&doc, "completion")?;
    let candidates = section(&doc, "candidates")?;

    Ok(Settings {
        engine: Options {
            completion: CompletionOptions {
                dynamic: boolean(completion, "completion", "dynamic")?,
                min_length: count(completion, "completion", "min_length")?,
                limit: count(completion, "completion", "limit")?,
                take_key: one_char(completion, "completion", "take_key")?,
            },
            candidates: CandidateOptions {
                until_list: count(candidates, "candidates", "until_list")?,
                labels: labels(candidates, "candidates", "labels")?,
            },
        },
        window: Window {
            show_reading: boolean(completion, "completion", "show_reading")?,
        },
    })
}

/// 雛形に無い項目を挙げる。
pub fn unknown_keys(text: &str) -> Vec<String> {
    let Ok(doc) = parse_document(text) else {
        return Vec::new();
    };
    let template = template();
    let mut unknown = Vec::new();
    for (section, item) in doc.as_table() {
        match (
            item.as_table(),
            template.get(section).and_then(Item::as_table),
        ) {
            (Some(present), Some(known)) => {
                for (key, _) in present {
                    if !known.contains_key(key) {
                        unknown.push(format!("{section}.{key}"));
                    }
                }
            }
            _ if template.contains_key(section) => {}
            _ => unknown.push(section.to_owned()),
        }
    }
    unknown
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

    let settings = parse(&filled.text)?;
    let unknown = unknown_keys(&filled.text);
    Ok(Loaded {
        settings,
        text: filled.text,
        added: filled.added,
        created: filled.created,
        unknown,
    })
}

/// 書きかけで途切れても元のファイルを壊さないよう、隣に書いてから置き換える。
fn write(path: &Path, text: &str) -> io::Result<()> {
    // 相対の名前だけなら親は空になる。作るものは無い。
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("toml.writing");
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

fn value<'a>(table: &'a Table, section: &str, key: &str) -> Result<&'a Item, Error> {
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

/// 一文字の文字列。
fn one_char(table: &Table, section: &str, key: &str) -> Result<char, Error> {
    let text = value(table, section, key)?.as_str();
    let mut chars = text.into_iter().flat_map(str::chars);
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_whitespace() => Ok(c),
        _ => Err(Error::new(format!(
            "{section}.{key} は一文字で書いてください (例: \".\")"
        ))),
    }
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
        let settings = parse(TEMPLATE).expect("雛形はそのまま使える");
        assert!(settings.engine.completion.dynamic);
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
        assert!(parse(&filled.text).is_ok());
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
        let user = "[completion]\ndynamic = false\nmin_length = 3\nlimit = 8\ntake_key = \",\"\nshow_reading = true\n";
        let filled = fill(Some(user)).unwrap();
        assert_eq!(filled.added, ["candidates.until_list", "candidates.labels"]);
        let settings = parse(&filled.text).unwrap();
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
        let settings = parse(&user).unwrap();
        assert_eq!(settings.engine.completion.min_length, 3);
    }

    #[test]
    fn nothing_is_taken_from_the_template_when_parsing() {
        // **読むときに雛形を見ない。** 足りなければ誤りになる。
        let user = without("limit = 16");
        let error = parse(&user).unwrap_err();
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
    fn broken_values_are_explained() {
        let cases = [
            ("labels = \"asdfjkl\"", "labels = \"\"", "candidates.labels"),
            ("labels = \"asdfjkl\"", "labels = \"asdfa\"", "二度"),
            ("take_key = \".\"", "take_key = \"..\"", "一文字"),
            ("min_length = 2", "min_length = 0", "1 以上"),
            ("dynamic = true", "dynamic = \"yes\"", "true か false"),
            ("until_list = 5", "until_list = 2.5", "整数"),
        ];
        for (from, to, expected) in cases {
            let user = TEMPLATE.replace(from, to);
            let error = parse(&user).expect_err(to);
            assert!(error.to_string().contains(expected), "{to}: {error}");
        }
    }

    #[test]
    fn a_section_written_as_a_value_is_refused() {
        let error = fill(Some("completion = 1\n")).unwrap_err();
        assert!(error.to_string().contains("[completion]"));
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
