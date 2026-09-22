//! 辞書ファイルの文字コード。
//!
//! CrystalSKK の内部表現とユーザー辞書は UTF-8 に統一する (ADR-0003)。
//! 配布されている SKK 辞書の多くは EUC-JP なので、読み込むときに一度だけ
//! 変換する。
//!
//! 変換は `encoding_rs` で行う。純 Rust の実装であり、C ツールチェインを
//! 要求しない (PRD N-08)。

/// 辞書のバイト列を復号した結果。
#[derive(Debug, Clone)]
pub struct Decoded {
    /// UTF-8 に直したテキスト。
    pub text: String,
    /// 元の文字コードの名前。
    pub encoding: &'static str,
    /// 復号できないバイトがあり、置換文字で埋めたか。
    pub had_errors: bool,
}

/// 辞書のバイト列を UTF-8 へ復号する。
///
/// 文字コードは次の順で決める。
///
/// 1. 先頭の注釈行にある `coding:` の指定
/// 2. UTF-8 として妥当ならば UTF-8
/// 3. それ以外は EUC-JP
///
/// SKK 辞書は先頭に `;; -*- coding: euc-jp -*-` を持つのが慣例なので、
/// 多くの場合 1 で決まる。
pub fn decode(bytes: &[u8]) -> Decoded {
    // UTF-8 の BOM だけは先に落とす。それ以外の BOM 判定は行わない。
    // EUC-JP の辞書の先頭バイトが偶然 UTF-16 の BOM と一致したときに、
    // 辞書全体を取り違えるのを避けるため。
    let bytes = bytes.strip_prefix(&UTF8_BOM).unwrap_or(bytes);

    if let Some(encoding) = coding_from_header(bytes) {
        return decode_with(bytes, encoding);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Decoded {
            text: text.to_owned(),
            encoding: "UTF-8",
            had_errors: false,
        },
        Err(_) => decode_with(bytes, encoding_rs::EUC_JP),
    }
}

fn decode_with(bytes: &[u8], encoding: &'static encoding_rs::Encoding) -> Decoded {
    let (text, had_errors) = encoding.decode_without_bom_handling(bytes);
    Decoded {
        text: text.into_owned(),
        encoding: encoding.name(),
        had_errors,
    }
}

/// UTF-8 のバイト順序記号。
const UTF8_BOM: [u8; 3] = [0xef, 0xbb, 0xbf];

/// 先頭数行の注釈から `coding:` の指定を読む。
fn coding_from_header(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    // 見出しの注釈は ASCII の範囲なので、そのままバイト列として探してよい。
    let head = &bytes[..bytes.len().min(HEADER_SCAN_BYTES)];
    let head = String::from_utf8_lossy(head);
    let line = head.lines().take(4).find(|l| l.contains("coding:"))?;
    let (_, rest) = line.split_once("coding:")?;
    let name = rest
        .trim_start()
        .split(|c: char| !c.is_ascii_graphic() || c == ';')
        .next()?;
    encoding_rs::Encoding::for_label(name.trim_end_matches("-*-").trim().as_bytes())
}

/// 文字コード指定を探す範囲。先頭の注釈だけ見れば十分。
const HEADER_SCAN_BYTES: usize = 512;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_utf8_as_is() {
        let decoded = decode("かんじ /漢字/".as_bytes());
        assert_eq!(decoded.text, "かんじ /漢字/");
        assert_eq!(decoded.encoding, "UTF-8");
        assert!(!decoded.had_errors);
    }

    #[test]
    fn falls_back_to_euc_jp() {
        let (bytes, _, _) = encoding_rs::EUC_JP.encode("かんじ /漢字/");
        let decoded = decode(&bytes);
        assert_eq!(decoded.text, "かんじ /漢字/");
        assert_eq!(decoded.encoding, "EUC-JP");
        assert!(!decoded.had_errors);
    }

    #[test]
    fn honours_the_coding_header() {
        let source = ";; -*- mode: fundamental; coding: euc-jp -*-\nかんじ /漢字/\n";
        let (bytes, _, _) = encoding_rs::EUC_JP.encode(source);
        let decoded = decode(&bytes);
        assert_eq!(decoded.encoding, "EUC-JP");
        assert!(decoded.text.contains("漢字"));
    }

    #[test]
    fn honours_a_utf8_coding_header() {
        let source = ";; -*- coding: utf-8 -*-\nかんじ /漢字/\n";
        let decoded = decode(source.as_bytes());
        assert_eq!(decoded.encoding, "UTF-8");
        assert!(decoded.text.contains("漢字"));
    }

    #[test]
    fn reports_undecodable_bytes_without_failing() {
        // EUC-JP としても UTF-8 としても壊れているバイト列。
        let decoded = decode(&[0xff, 0xfe, 0x41]);
        assert!(decoded.had_errors);
        assert!(decoded.text.ends_with('A'), "読める部分は残す");
    }

    #[test]
    fn strips_a_utf8_bom() {
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        bytes.extend_from_slice("かんじ /漢字/".as_bytes());
        let decoded = decode(&bytes);
        assert_eq!(decoded.text, "かんじ /漢字/");
        assert_eq!(decoded.encoding, "UTF-8");
    }
}
