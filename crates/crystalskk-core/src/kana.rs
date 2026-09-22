//! かな・カナ・英数の字種変換。
//!
//! すべて純粋関数。入力モードによる出し分けは [`crate::mode`] が行う。

/// ひらがなをカタカナに変換する。ひらがな以外はそのまま通す。
pub fn to_katakana(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            // ぁ(U+3041)〜ゖ(U+3096) は ァ(U+30A1)〜ヶ(U+30F6) と同じ並び。
            'ぁ'..='ゖ' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// カタカナをひらがなに変換する。カタカナ以外はそのまま通す。
pub fn to_hiragana(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ァ'..='ヶ' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// ひらがな・カタカナを半角カタカナに変換する。
///
/// 濁点・半濁点は独立した文字に分解される (`が` → `ｶﾞ`) ため、
/// 文字数は入力より増えうる。
pub fn to_halfwidth_katakana(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in to_katakana(s).chars() {
        match halfwidth_of(c) {
            Some(h) => out.push_str(h),
            None => out.push(c),
        }
    }
    out
}

#[rustfmt::skip]
fn halfwidth_of(c: char) -> Option<&'static str> {
    let s = match c {
        'ァ' => "ｧ", 'ィ' => "ｨ", 'ゥ' => "ｩ", 'ェ' => "ｪ", 'ォ' => "ｫ",
        'ア' => "ｱ", 'イ' => "ｲ", 'ウ' => "ｳ", 'エ' => "ｴ", 'オ' => "ｵ",
        'カ' => "ｶ", 'キ' => "ｷ", 'ク' => "ｸ", 'ケ' => "ｹ", 'コ' => "ｺ",
        'ガ' => "ｶﾞ", 'ギ' => "ｷﾞ", 'グ' => "ｸﾞ", 'ゲ' => "ｹﾞ", 'ゴ' => "ｺﾞ",
        'サ' => "ｻ", 'シ' => "ｼ", 'ス' => "ｽ", 'セ' => "ｾ", 'ソ' => "ｿ",
        'ザ' => "ｻﾞ", 'ジ' => "ｼﾞ", 'ズ' => "ｽﾞ", 'ゼ' => "ｾﾞ", 'ゾ' => "ｿﾞ",
        'タ' => "ﾀ", 'チ' => "ﾁ", 'ツ' => "ﾂ", 'テ' => "ﾃ", 'ト' => "ﾄ",
        'ダ' => "ﾀﾞ", 'ヂ' => "ﾁﾞ", 'ヅ' => "ﾂﾞ", 'デ' => "ﾃﾞ", 'ド' => "ﾄﾞ",
        'ッ' => "ｯ",
        'ナ' => "ﾅ", 'ニ' => "ﾆ", 'ヌ' => "ﾇ", 'ネ' => "ﾈ", 'ノ' => "ﾉ",
        'ハ' => "ﾊ", 'ヒ' => "ﾋ", 'フ' => "ﾌ", 'ヘ' => "ﾍ", 'ホ' => "ﾎ",
        'バ' => "ﾊﾞ", 'ビ' => "ﾋﾞ", 'ブ' => "ﾌﾞ", 'ベ' => "ﾍﾞ", 'ボ' => "ﾎﾞ",
        'パ' => "ﾊﾟ", 'ピ' => "ﾋﾟ", 'プ' => "ﾌﾟ", 'ペ' => "ﾍﾟ", 'ポ' => "ﾎﾟ",
        'マ' => "ﾏ", 'ミ' => "ﾐ", 'ム' => "ﾑ", 'メ' => "ﾒ", 'モ' => "ﾓ",
        'ャ' => "ｬ", 'ュ' => "ｭ", 'ョ' => "ｮ",
        'ヤ' => "ﾔ", 'ユ' => "ﾕ", 'ヨ' => "ﾖ",
        'ラ' => "ﾗ", 'リ' => "ﾘ", 'ル' => "ﾙ", 'レ' => "ﾚ", 'ロ' => "ﾛ",
        'ヮ' => "ﾜ", 'ワ' => "ﾜ", 'ヰ' => "ｲ", 'ヱ' => "ｴ", 'ヲ' => "ｦ",
        'ン' => "ﾝ",
        'ヴ' => "ｳﾞ",
        'ー' => "ｰ", '、' => "､", '。' => "｡", '「' => "｢", '」' => "｣", '・' => "･",
        '　' => " ",
        _ => return None,
    };
    Some(s)
}

/// ASCII を全角英数に変換する。
pub fn to_fullwidth_ascii(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => '　',
            '!'..='~' => char::from_u32(c as u32 + 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn katakana_roundtrip() {
        assert_eq!(to_katakana("あいうえおっゃ"), "アイウエオッャ");
        assert_eq!(to_hiragana("アイウエオッャ"), "あいうえおっゃ");
        // ヴ にはひらがな ゔ (U+3094) が対応する。
        assert_eq!(to_hiragana("ヴ"), "ゔ");
        assert_eq!(to_katakana("ゔ"), "ヴ");
    }

    #[test]
    fn non_kana_passes_through() {
        assert_eq!(to_katakana("abc123漢字"), "abc123漢字");
    }

    #[test]
    fn halfwidth_decomposes_dakuten() {
        assert_eq!(to_halfwidth_katakana("がぎぐげご"), "ｶﾞｷﾞｸﾞｹﾞｺﾞ");
        assert_eq!(to_halfwidth_katakana("ぱんつ"), "ﾊﾟﾝﾂ");
        assert_eq!(to_halfwidth_katakana("しゃっきり"), "ｼｬｯｷﾘ");
        assert_eq!(to_halfwidth_katakana("ヴァイオリン"), "ｳﾞｧｲｵﾘﾝ");
    }

    #[test]
    fn halfwidth_keeps_unknown() {
        assert_eq!(to_halfwidth_katakana("漢字ﾃｽﾄ"), "漢字ﾃｽﾄ");
    }

    #[test]
    fn fullwidth_ascii() {
        assert_eq!(to_fullwidth_ascii("Abc 123!"), "Ａｂｃ　１２３！");
    }
}
