//! TIP と辞書サーバが交わす語彙。
//!
//! 辞書は**サーバだけが持つ** (ADR-0016)。TIP は引きたいものを頼み、
//! 返ってきたものを使う。この crate はその頼み方と答え方だけを定める。
//!
//! # 何も知らない crate である
//!
//! パイプも Windows も出てこない。**文字列にする／文字列から読む**、
//! それだけを担う。おかげで運び方 (名前付きパイプ) を後から変えても、
//! 語彙は動かさずに済む。試験も普通に書ける。
//!
//! # 形
//!
//! 一つの頼みは一行である。欄は水平タブで区切る。
//!
//! ```text
//! search\t<見出し語>\t<送り仮名>
//! convert\t<見出し語>\t<送り仮名>\t<前の文章>\t<後ろの文章>
//! learn\t<見出し語>\t<送り仮名>\t<語>
//! settings
//! reset\t<settings か romaji>
//! open-folder
//! save
//! exit
//! ```
//!
//! 答えも一行。
//!
//! ```text
//! ok\t<候補>\u{1f}<候補>...
//! settings\t<設定ファイルの全文>\u{1e}<ローマ字テーブルの全文>
//! done\t<したこと>
//! error\t<訳>
//! ```
//!
//! `convert` は変換のための `search` で、カーソルの前後の文章を添える。
//! サーバはそれを見て候補を並べる (ADR-0030)。前後の文章は画面にある
//! 任意の文字列なので、**区切りに使う文字 (タブ・改行・`\u{1f}`・
//! `\u{1e}`) は空白にしてから運ぶ。** 並べるための手がかりなので、
//! その程度の崩れは構わない。
//!
//! 候補と候補は `\u{1f}`、語と注釈は `\u{1e}` で分ける。**辞書の中身に
//! 現れない文字**なので、逃がし方を決めずに済む。SKK 辞書はこれらの制御
//! 文字を含まない。
//!
//! 設定ファイルとローマ字テーブルの改行も `\u{1f}` にし、二つの間は
//! `\u{1e}` で分ける。TOML は字下げ以外の制御文字をそのまま書くことを
//! 許さず、ローマ字テーブルも読むときに弾くので、**読めたファイルには
//! 現れない。** 字下げ (タブ) は欄の区切りと同じ文字だが、全文は一行の
//! 残り全部として読むので混ざらない。

use crystalskk_core::dict::{Candidate, Query};

/// 欄の区切り。
const FIELD: char = '\t';

/// 候補と候補の区切り。
const BETWEEN_CANDIDATES: char = '\u{1f}';

/// 語と注釈の区切り。
const WITHIN_CANDIDATE: char = '\u{1e}';

/// 設定ファイルの改行の代わり。
const LINE_BREAK: char = '\u{1f}';

/// 設定ファイルとローマ字テーブルの区切り。
const BETWEEN_FILES: char = '\u{1e}';

/// 雛形に戻すもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reset {
    /// 設定ファイル。
    Settings,
    /// ローマ字テーブル。
    Romaji,
}

/// 設定ファイルが変わったという知らせの名前 (ADR-0040)。
///
/// サーバはこの名前で Windows に番号を振ってもらい (`RegisterWindowMessageW`)、
/// 全ウィンドウへ送る。TIP も同じ名前で番号を得て、受けたら設定を取り直す。
/// **名前が同じなら、どのプロセスでも番号は同じになる。**
pub const SETTINGS_CHANGED_MESSAGE: &str = "CrystalSKK.SettingsChanged";

/// TIP からサーバへの頼み。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// 見出し語を引く。
    Search(Query),
    /// 変換のために見出し語を引く。カーソルの前後の文章を添え、サーバは
    /// それを見て候補を並べる。取れなかった文章は空。
    Convert {
        query: Query,
        before: String,
        after: String,
    },
    /// 前方一致する見出しを引く。補完に使う。
    Complete { prefix: String, limit: usize },
    /// 選ばれた候補を覚える。並び順の学習に使う。
    Learn { query: Query, word: String },
    /// 新しい語を登録する。
    Register { query: Query, word: String },
    /// 候補をユーザー辞書から消す。
    Purge { query: Query, word: String },
    /// 設定を尋ねる。
    ///
    /// **設定ファイルを読むのもサーバである。** 足りない項目を書き足す
    /// ことがあり、書き手は一人に絞りたい。隔離された入れ物の中の TIP
    /// からは、そもそもファイルが読めない。
    Settings,
    /// 雛形で上書きする。元の中身は退避する。
    Reset(Reset),
    /// 設定ファイルの置き場所を開く。
    ///
    /// **開くのもサーバである。** 隔離された入れ物の中からは、エクス
    /// プローラーを立ち上げられないことがある。
    OpenFolder,
    /// 設定ファイルが変わったと、全ウィンドウへ知らせる (ADR-0040)。
    ///
    /// ふだんはサーバがファイルを見張って自分で知らせる。これは利用者が
    /// 「設定を検査する」を選んだときの頼みで、見張りの取りこぼしへの
    /// 備えを兼ねる。**知らせるのもサーバである。** 隔離された入れ物の中の
    /// TIP から送っても、ふつうのアプリの窓には届かない。
    Announce,
    /// ユーザー辞書を書き出す。
    Save,
    /// 終わる。
    ///
    /// **落とすのではなく頼む。** サーバはユーザー辞書の唯一の書き手なので、
    /// 強制終了は書きかけの学習を捨てることになる。
    Exit,
}

/// サーバから TIP への答え。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// 引けた候補。頼みが検索でなければ空。
    Ok(Vec<Candidate>),
    /// 設定ファイルとローマ字テーブルの全文。設定ファイルの足りない項目は
    /// 書き足してある。
    Settings { config: String, romaji: String },
    /// 頼まれたことをした。利用者に見せる文を添える。
    Done(String),
    /// できなかった。
    Error(String),
}

impl Request {
    /// 一行の文字列にする。
    pub fn encode(&self) -> String {
        match self {
            Self::Search(query) => format!("search{FIELD}{}", encode_query(query)),
            Self::Convert {
                query,
                before,
                after,
            } => format!(
                "convert{FIELD}{}{FIELD}{}{FIELD}{}",
                encode_query(query),
                plain(before),
                plain(after)
            ),
            Self::Complete { prefix, limit } => format!("complete{FIELD}{prefix}{FIELD}{limit}"),
            Self::Learn { query, word } => {
                format!("learn{FIELD}{}{FIELD}{word}", encode_query(query))
            }
            Self::Register { query, word } => {
                format!("register{FIELD}{}{FIELD}{word}", encode_query(query))
            }
            Self::Purge { query, word } => {
                format!("purge{FIELD}{}{FIELD}{word}", encode_query(query))
            }
            Self::Settings => "settings".to_owned(),
            Self::Reset(Reset::Settings) => format!("reset{FIELD}settings"),
            Self::Reset(Reset::Romaji) => format!("reset{FIELD}romaji"),
            Self::OpenFolder => "open-folder".to_owned(),
            Self::Announce => "announce".to_owned(),
            Self::Save => "save".to_owned(),
            Self::Exit => "exit".to_owned(),
        }
    }

    /// 一行の文字列から読む。
    ///
    /// 読めない行は `None`。**知らない頼みを黙って別の頼みとして扱わない。**
    pub fn decode(line: &str) -> Option<Self> {
        let line = line.trim_end_matches(['\r', '\n']);
        let mut fields = line.split(FIELD);
        match fields.next()? {
            "search" => Some(Self::Search(decode_query(&mut fields)?)),
            "convert" => {
                let query = decode_query(&mut fields)?;
                Some(Self::Convert {
                    query,
                    before: fields.next()?.to_owned(),
                    after: fields.next()?.to_owned(),
                })
            }
            "complete" => Some(Self::Complete {
                prefix: fields.next()?.to_owned(),
                limit: fields.next()?.parse().ok()?,
            }),
            "learn" => {
                let query = decode_query(&mut fields)?;
                Some(Self::Learn {
                    query,
                    word: fields.next()?.to_owned(),
                })
            }
            "register" => {
                let query = decode_query(&mut fields)?;
                Some(Self::Register {
                    query,
                    word: fields.next()?.to_owned(),
                })
            }
            "purge" => {
                let query = decode_query(&mut fields)?;
                Some(Self::Purge {
                    query,
                    word: fields.next()?.to_owned(),
                })
            }
            "settings" => Some(Self::Settings),
            "reset" => match fields.next()? {
                "settings" => Some(Self::Reset(Reset::Settings)),
                "romaji" => Some(Self::Reset(Reset::Romaji)),
                _ => None,
            },
            "open-folder" => Some(Self::OpenFolder),
            "announce" => Some(Self::Announce),
            "save" => Some(Self::Save),
            "exit" => Some(Self::Exit),
            _ => None,
        }
    }
}

impl Response {
    /// 一行の文字列にする。
    pub fn encode(&self) -> String {
        match self {
            Self::Ok(candidates) => {
                let body = candidates
                    .iter()
                    .map(encode_candidate)
                    .collect::<Vec<_>>()
                    .join(&BETWEEN_CANDIDATES.to_string());
                format!("ok{FIELD}{body}")
            }
            Self::Settings { config, romaji } => {
                format!(
                    "settings{FIELD}{}{BETWEEN_FILES}{}",
                    one_line(config),
                    one_line(romaji)
                )
            }
            Self::Done(what) => format!("done{FIELD}{what}"),
            Self::Error(reason) => format!("error{FIELD}{reason}"),
        }
    }

    /// 一行の文字列から読む。
    pub fn decode(line: &str) -> Option<Self> {
        let line = line.trim_end_matches(['\r', '\n']);
        let (kind, body) = match line.split_once(FIELD) {
            Some(split) => split,
            // 欄が一つだけの答えもある。候補ゼロの `ok` がそれ。
            None => (line, ""),
        };
        match kind {
            "ok" if body.is_empty() => Some(Self::Ok(Vec::new())),
            "ok" => Some(Self::Ok(
                body.split(BETWEEN_CANDIDATES)
                    .map(decode_candidate)
                    .collect(),
            )),
            "settings" => {
                let (config, romaji) = body.split_once(BETWEEN_FILES)?;
                Some(Self::Settings {
                    config: config.replace(LINE_BREAK, "\n"),
                    romaji: romaji.replace(LINE_BREAK, "\n"),
                })
            }
            "done" => Some(Self::Done(body.to_owned())),
            "error" => Some(Self::Error(body.to_owned())),
            _ => None,
        }
    }
}

/// 区切りに使う文字を空白にする。
fn plain(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            FIELD | '\r' | '\n' | BETWEEN_CANDIDATES | WITHIN_CANDIDATE => ' ',
            c => c,
        })
        .collect()
}

/// 全文を一行にする。
fn one_line(text: &str) -> String {
    text.replace('\r', "")
        .replace('\n', &LINE_BREAK.to_string())
}

/// 見出し語と送り仮名を二つの欄にする。
fn encode_query(query: &Query) -> String {
    format!(
        "{}{FIELD}{}",
        query.key,
        query.okuri.as_deref().unwrap_or_default()
    )
}

/// 二つの欄から見出し語を組み立てる。
fn decode_query<'a>(fields: &mut impl Iterator<Item = &'a str>) -> Option<Query> {
    let key = fields.next()?.to_owned();
    let okuri = fields.next()?;
    // 空の欄は「送り仮名なし」。空文字列の送り仮名とは区別する。
    Some(Query {
        key,
        okuri: (!okuri.is_empty()).then(|| okuri.to_owned()),
    })
}

fn encode_candidate(candidate: &Candidate) -> String {
    match &candidate.annotation {
        Some(annotation) => format!("{}{WITHIN_CANDIDATE}{annotation}", candidate.word),
        None => candidate.word.clone(),
    }
}

fn decode_candidate(text: &str) -> Candidate {
    match text.split_once(WITHIN_CANDIDATE) {
        Some((word, annotation)) => Candidate::with_annotation(word, annotation),
        None => Candidate::new(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(request: &Request) {
        let line = request.encode();
        assert!(!line.contains('\n'), "一つの頼みは一行: {line:?}");
        assert_eq!(Request::decode(&line).as_ref(), Some(request));
    }

    #[test]
    fn requests_survive_a_round_trip() {
        let okuri_nashi = Query::okuri_nashi("かんじ");
        let okuri_ari = Query::okuri_ari("おく", 'r', "り");

        roundtrip(&Request::Search(okuri_nashi.clone()));
        roundtrip(&Request::Search(okuri_ari.clone()));
        roundtrip(&Request::Learn {
            query: okuri_ari,
            word: "送".to_owned(),
        });
        roundtrip(&Request::Register {
            query: okuri_nashi.clone(),
            word: "漢字".to_owned(),
        });
        roundtrip(&Request::Purge {
            query: okuri_nashi,
            word: "漢字".to_owned(),
        });
        roundtrip(&Request::Save);
        roundtrip(&Request::Exit);
    }

    #[test]
    fn a_conversion_carries_the_surroundings() {
        roundtrip(&Request::Convert {
            query: Query::okuri_ari("おく", 'r', "り"),
            before: "荷物を".to_owned(),
            after: "ます".to_owned(),
        });
        // 前後が取れなかった変換も運べる。
        roundtrip(&Request::Convert {
            query: Query::okuri_nashi("かんじ"),
            before: String::new(),
            after: String::new(),
        });
    }

    #[test]
    fn separators_in_the_surroundings_become_spaces() {
        let request = Request::Convert {
            query: Query::okuri_nashi("かんじ"),
            before: "一行目\r\n\t二行目\u{1f}".to_owned(),
            after: "\u{1e}後".to_owned(),
        };
        let line = request.encode();
        assert!(!line.contains('\n'));
        assert_eq!(
            Request::decode(&line),
            Some(Request::Convert {
                query: Query::okuri_nashi("かんじ"),
                before: "一行目   二行目 ".to_owned(),
                after: " 後".to_owned(),
            })
        );
    }

    #[test]
    fn settings_requests_survive_a_round_trip() {
        roundtrip(&Request::Settings);
        roundtrip(&Request::Reset(Reset::Settings));
        roundtrip(&Request::Reset(Reset::Romaji));
        roundtrip(&Request::OpenFolder);
        roundtrip(&Request::Announce);
    }

    #[test]
    fn an_unknown_reset_is_refused() {
        assert_eq!(Request::decode("reset\teverything"), None);
    }

    #[test]
    fn both_files_travel_on_one_line() {
        // 一つの答えは一行。**改行やタブを含む全文でも、一行で運ぶ。**
        let response = Response::Settings {
            config: "[completion]\n# 補完候補\ndynamic = true\n\tlimit = 16\n".to_owned(),
            romaji: "# 説明\nka\tか\nkk\tっ\tk\n".to_owned(),
        };
        let line = response.encode();
        assert!(!line.contains('\n'));
        assert_eq!(Response::decode(&line), Some(response));
    }

    #[test]
    fn what_was_done_is_told() {
        let response = Response::Done("上書きしました".to_owned());
        assert_eq!(Response::decode(&response.encode()), Some(response));
    }

    #[test]
    fn a_completion_request_survives_a_round_trip() {
        roundtrip(&Request::Complete {
            prefix: "かん".to_owned(),
            limit: 16,
        });
    }

    #[test]
    fn an_empty_okuri_field_means_no_okuri() {
        let line = Request::Search(Query::okuri_nashi("かんじ")).encode();
        let Some(Request::Search(query)) = Request::decode(&line) else {
            panic!("読める");
        };
        assert_eq!(query.okuri, None, "空の欄と「空の送り仮名」を混ぜない");
    }

    #[test]
    fn responses_survive_a_round_trip() {
        let candidates = vec![
            Candidate::new("漢字"),
            Candidate::with_annotation("感じ", "feeling"),
        ];
        let line = Response::Ok(candidates.clone()).encode();
        assert_eq!(Response::decode(&line), Some(Response::Ok(candidates)));
    }

    #[test]
    fn no_candidates_is_not_an_error() {
        let line = Response::Ok(Vec::new()).encode();
        assert_eq!(Response::decode(&line), Some(Response::Ok(Vec::new())));
    }

    #[test]
    fn errors_carry_their_reason() {
        let line = Response::Error("辞書がありません".to_owned()).encode();
        assert_eq!(
            Response::decode(&line),
            Some(Response::Error("辞書がありません".to_owned()))
        );
    }

    #[test]
    fn an_unknown_request_is_refused_not_guessed() {
        // 知らない頼みを別の頼みとして扱うと、**辞書を壊す頼みに化けうる**。
        assert_eq!(Request::decode("erase\tかんじ\t"), None);
        assert_eq!(Request::decode(""), None);
    }

    #[test]
    fn a_word_with_an_annotation_keeps_both() {
        let candidate = Candidate::with_annotation("橋", "bridge");
        let line = Response::Ok(vec![candidate.clone()]).encode();
        let Some(Response::Ok(decoded)) = Response::decode(&line) else {
            panic!("読める");
        };
        assert_eq!(decoded, vec![candidate]);
    }
}
