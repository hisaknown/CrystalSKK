//! SKK 辞書のテキスト形式。
//!
//! 一行が一件の見出しを表す。
//!
//! ```text
//! かんじ /漢字/感じ;feeling/幹事/
//! おくr /送/贈/
//! ```
//!
//! 候補に `/` や `;` を含めたいときは Emacs Lisp の `concat` 形式で書く。
//!
//! ```text
//! ab /(concat "a\057b")/
//! ```
//!
//! この形式は他の SKK 実装と共有される資産なので、読み書きともに
//! 崩さないことを最優先する (PRD N-10)。

use crystalskk_core::Candidate;

/// 辞書の一行を見出しと候補に分解する。
///
/// 注釈行・空行なら `None`。壊れた行も `None` を返して読み飛ばす。
/// 辞書は外部から来るものなので、一行の破損で辞書全体を失わせない (PRD N-09)。
pub fn parse_line(line: &str) -> Option<(String, Vec<Candidate>)> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() || line.starts_with(';') {
        return None;
    }

    let (key, rest) = line.split_once(' ')?;
    if key.is_empty() {
        return None;
    }

    let candidates = parse_candidates(rest);
    if candidates.is_empty() {
        return None;
    }
    Some((key.to_owned(), candidates))
}

/// `/候補1/候補2;注釈/` の部分を候補列に分解する。
pub fn parse_candidates(field: &str) -> Vec<Candidate> {
    field
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|entry| match entry.split_once(';') {
            Some((word, annotation)) => {
                Candidate::with_annotation(decode_word(word), decode_word(annotation))
            }
            None => Candidate::new(decode_word(entry)),
        })
        .collect()
}

/// 見出しと候補を辞書の一行に組み立てる。末尾に改行は付けない。
pub fn format_line(key: &str, candidates: &[Candidate]) -> String {
    let mut out = String::with_capacity(key.len() + candidates.len() * 8);
    out.push_str(key);
    out.push(' ');
    out.push('/');
    for candidate in candidates {
        out.push_str(&encode_word(&candidate.word));
        if let Some(annotation) = &candidate.annotation {
            out.push(';');
            out.push_str(&encode_word(annotation));
        }
        out.push('/');
    }
    out
}

/// `concat` 形式なら展開する。そうでなければそのまま。
fn decode_word(word: &str) -> String {
    let Some(inner) = word
        .strip_prefix("(concat \"")
        .and_then(|s| s.strip_suffix("\")"))
    else {
        return word.to_owned();
    };

    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            // 八進数エスケープ。`\057` が `/`。
            Some(d) if d.is_digit(8) => {
                let mut value: u32 = 0;
                for _ in 0..3 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(digit) => {
                            value = value * 8 + digit;
                            chars.next();
                        }
                        None => break,
                    }
                }
                match char::from_u32(value) {
                    Some(c) => out.push(c),
                    None => return word.to_owned(),
                }
            }
            Some(_) => out.push(chars.next().expect("peek した文字がある")),
            // 末尾の孤立した `\` は元の表記のまま扱う。
            None => return word.to_owned(),
        }
    }
    out
}

/// 区切り文字を含む語を `concat` 形式にする。含まなければそのまま。
fn encode_word(word: &str) -> String {
    if !word.contains(['/', ';', '"', '\\']) {
        return word.to_owned();
    }
    let mut inner = String::with_capacity(word.len() + 8);
    for c in word.chars() {
        match c {
            '/' => inner.push_str("\\057"),
            ';' => inner.push_str("\\073"),
            '"' => inner.push_str("\\\""),
            '\\' => inner.push_str("\\\\"),
            c => inner.push(c),
        }
    }
    format!("(concat \"{inner}\")")
}

/// 見出しの形から送りありらしさを推測する。末尾が ASCII 英字なら送りあり。
///
/// **推測でしかない。** abbrev の見出し (`skk`) も同じ形になるため、
/// 区切りの注釈行 (`;; okuri-ari entries.`) がある辞書では必ずそちらを
/// 信じること。この関数は区切りのない辞書を読むときの最後の手段である。
pub fn is_okuri_ari_key(key: &str) -> bool {
    key.chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_entry() {
        let (key, candidates) = parse_line("かんじ /漢字/感じ/幹事/").expect("読めるはず");
        assert_eq!(key, "かんじ");
        let words: Vec<&str> = candidates.iter().map(|c| c.word.as_str()).collect();
        assert_eq!(words, ["漢字", "感じ", "幹事"]);
        assert!(candidates.iter().all(|c| c.annotation.is_none()));
    }

    #[test]
    fn parses_annotations() {
        let (_, candidates) = parse_line("かんじ /漢字/感じ;feeling/").expect("読めるはず");
        assert_eq!(candidates[1].word, "感じ");
        assert_eq!(candidates[1].annotation.as_deref(), Some("feeling"));
    }

    #[test]
    fn skips_comments_and_blank_lines() {
        assert!(parse_line(";; okuri-ari entries.").is_none());
        assert!(parse_line("").is_none());
        assert!(parse_line("   ").is_none());
    }

    #[test]
    fn skips_broken_lines_instead_of_failing() {
        assert!(parse_line("見出しだけ").is_none());
        assert!(parse_line("みだし //").is_none());
    }

    #[test]
    fn decodes_concat_escapes() {
        let (_, candidates) = parse_line(r#"ab /(concat "a\057b")/"#).expect("読めるはず");
        assert_eq!(candidates[0].word, "a/b");

        let (_, candidates) = parse_line(r#"x /(concat "a\073b")/"#).expect("読めるはず");
        assert_eq!(candidates[0].word, "a;b");
    }

    #[test]
    fn leaves_non_concat_words_alone() {
        let (_, candidates) = parse_line("x /(a b)/").expect("読めるはず");
        assert_eq!(candidates[0].word, "(a b)");
    }

    #[test]
    fn formats_back_to_the_same_line() {
        let line = "かんじ /漢字/感じ;feeling/幹事/";
        let (key, candidates) = parse_line(line).expect("読めるはず");
        assert_eq!(format_line(&key, &candidates), line);
    }

    #[test]
    fn round_trips_words_needing_escapes() {
        let original = Candidate::new("a/b;c");
        let line = format_line("x", std::slice::from_ref(&original));
        let (_, candidates) = parse_line(&line).expect("読めるはず");
        assert_eq!(candidates[0], original);
    }

    #[test]
    fn guesses_okuri_ari_from_the_key_shape() {
        assert!(is_okuri_ari_key("おくr"));
        assert!(!is_okuri_ari_key("かんじ"));
        // abbrev の見出しと区別が付かない。だから区切りの注釈行が優先される。
        assert!(is_okuri_ari_key("skk"));
    }
}
