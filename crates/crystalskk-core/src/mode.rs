//! 入力モード。
//!
//! SKK のモードは「ローマ字かな変換を通すか」と「通した結果をどの字種で
//! 出すか」の二軸で決まる。[`InputMode::Ascii`] だけがかな変換を通さない。

use crate::kana;

/// 入力モード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InputMode {
    /// ひらがな (`C-j`)。SKK の既定。
    #[default]
    Hiragana,
    /// カタカナ (`q`)。
    Katakana,
    /// 半角カタカナ (`C-q`)。
    HalfKatakana,
    /// 全角英数 (`L`)。かな変換は通さず、打鍵をそのまま全角にする。
    FullAscii,
    /// 半角英数 (`l`)。IME が素通しになる状態。
    Ascii,
}

impl InputMode {
    /// かな変換を通すモードか。
    pub fn is_kana(self) -> bool {
        matches!(self, Self::Hiragana | Self::Katakana | Self::HalfKatakana)
    }

    /// 変換で得られたひらがなを、このモードの字種に直す。
    ///
    /// かな以外のモードではひらがなをそのまま返す。見出し語の入力中は
    /// モードに関わらずひらがなで保持するため、その用途では呼ばない。
    pub fn render_kana(self, hiragana: &str) -> String {
        match self {
            Self::Hiragana => hiragana.to_owned(),
            Self::Katakana => kana::to_katakana(hiragana),
            Self::HalfKatakana => kana::to_halfwidth_katakana(hiragana),
            Self::FullAscii | Self::Ascii => hiragana.to_owned(),
        }
    }

    /// モード表示用の短い名前。言語バーや候補ウィンドウで使う。
    pub fn label(self) -> &'static str {
        match self {
            Self::Hiragana => "あ",
            Self::Katakana => "ア",
            Self::HalfKatakana => "ｱ",
            Self::FullAscii => "Ａ",
            Self::Ascii => "A",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kana_modes_render_their_own_script() {
        assert_eq!(InputMode::Hiragana.render_kana("かんじ"), "かんじ");
        assert_eq!(InputMode::Katakana.render_kana("かんじ"), "カンジ");
        assert_eq!(InputMode::HalfKatakana.render_kana("かんじ"), "ｶﾝｼﾞ");
    }

    #[test]
    fn ascii_modes_are_not_kana_modes() {
        assert!(InputMode::Hiragana.is_kana());
        assert!(!InputMode::Ascii.is_kana());
        assert!(!InputMode::FullAscii.is_kana());
    }
}
