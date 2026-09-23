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
//! learn\t<見出し語>\t<送り仮名>\t<語>
//! settings
//! save
//! exit
//! ```
//!
//! 答えも一行。
//!
//! ```text
//! ok\t<候補>\u{1f}<候補>...
//! settings\t<設定ファイルの全文。改行は \u{1f}>
//! error\t<訳>
//! ```
//!
//! 候補と候補は `\u{1f}`、語と注釈は `\u{1e}` で分ける。**辞書の中身に
//! 現れない文字**なので、逃がし方を決めずに済む。SKK 辞書はこれらの制御
//! 文字を含まない。
//!
//! 設定ファイルの改行も `\u{1f}` にする。TOML は字下げ以外の制御文字を
//! そのまま書くことを許さないので、**読めた設定ファイルには現れない。**

use crystalskk_core::dict::{Candidate, Query};

/// 欄の区切り。
const FIELD: char = '\t';

/// 候補と候補の区切り。
const BETWEEN_CANDIDATES: char = '\u{1f}';

/// 語と注釈の区切り。
const WITHIN_CANDIDATE: char = '\u{1e}';

/// 設定ファイルの改行の代わり。
const LINE_BREAK: char = '\u{1f}';

/// TIP からサーバへの頼み。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// 見出し語を引く。
    Search(Query),
    /// 前方一致する見出しを引く。補完に使う。
    Complete { prefix: String, limit: usize },
    /// 選ばれた候補を覚える。並び順の学習に使う。
    Learn { query: Query, word: String },
    /// 新しい語を登録する。
    Register { query: Query, word: String },
    /// 設定を尋ねる。
    ///
    /// **設定ファイルを読むのもサーバである。** 足りない項目を書き足す
    /// ことがあり、書き手は一人に絞りたい。隔離された入れ物の中の TIP
    /// からは、そもそもファイルが読めない。
    Settings,
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
    /// 設定ファイルの全文。足りない項目は書き足してある。
    Settings(String),
    /// できなかった。
    Error(String),
}

impl Request {
    /// 一行の文字列にする。
    pub fn encode(&self) -> String {
        match self {
            Self::Search(query) => format!("search{FIELD}{}", encode_query(query)),
            Self::Complete { prefix, limit } => format!("complete{FIELD}{prefix}{FIELD}{limit}"),
            Self::Learn { query, word } => {
                format!("learn{FIELD}{}{FIELD}{word}", encode_query(query))
            }
            Self::Register { query, word } => {
                format!("register{FIELD}{}{FIELD}{word}", encode_query(query))
            }
            Self::Settings => "settings".to_owned(),
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
            "settings" => Some(Self::Settings),
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
            Self::Settings(text) => {
                let body = text
                    .replace('\r', "")
                    .replace('\n', &LINE_BREAK.to_string());
                format!("settings{FIELD}{body}")
            }
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
            "settings" => Some(Self::Settings(body.replace(LINE_BREAK, "\n"))),
            "error" => Some(Self::Error(body.to_owned())),
            _ => None,
        }
    }
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
            query: okuri_nashi,
            word: "漢字".to_owned(),
        });
        roundtrip(&Request::Save);
        roundtrip(&Request::Exit);
    }

    #[test]
    fn a_settings_request_survives_a_round_trip() {
        roundtrip(&Request::Settings);
    }

    #[test]
    fn the_settings_file_travels_on_one_line() {
        // 一つの答えは一行。**改行を含む全文でも、一行で運ぶ。**
        let text = "[completion]\n# 補完候補\ndynamic = true\n\tlimit = 16\n";
        let response = Response::Settings(text.to_owned());
        let line = response.encode();
        assert!(!line.contains('\n'));
        assert_eq!(Response::decode(&line), Some(response));
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
