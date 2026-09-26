//! エンジンの振る舞いを決める値。
//!
//! **既定値を持たない。** ここにある値はすべて利用者の設定ファイルから
//! 来る (ADR-0020)。`Default` を実装しないのはそのためで、実装すれば
//! どこかで黙って使われ、ファイルに書かれていない値が効くことになる。
//!
//! 値の正しさ (空でない、重ならない、など) は読み込む側が確かめてから
//! 渡す。**ここに来た時点で使える値である。**

use crate::keymap::Keymap;
use crate::romaji::RomajiTable;

/// エンジンの振る舞いを決める値の一式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub completion: CompletionOptions,
    pub candidates: CandidateOptions,
    /// ローマ字の規則表。利用者のファイルから読んだもの (ADR-0021)。
    pub romaji: RomajiTable,
    /// キーの割り当て (ADR-0038)。
    pub keys: Keymap,
}

/// 補完。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionOptions {
    /// 打っている最中に補完候補を出すか。
    ///
    /// 切っても Tab の補完は使える。そちらは利用者が呼ぶものである。
    pub dynamic: bool,
    /// 補完を始める見出し語の長さ。
    pub min_length: usize,
    /// 一度に覚えておく補完の数。Tab はこの範囲を巡る。
    pub limit: usize,
}

/// 候補の並べ方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateOptions {
    /// 何回目の変換から一覧に移るか。
    pub until_list: usize,
    /// 一覧から候補を選ぶキー。この数が一ページの候補の数になる。
    pub labels: Vec<char>,
}

/// 候補を一覧にするときの区切り方。
///
/// 候補選択の状態と、外へ見せる一覧の両方が同じ区切り方を使う。**片方だけ
/// 数え方が違うと、見えているページと選べる候補がずれる。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    labels: Vec<char>,
    /// 一覧に載る最初の候補の位置。
    first_listed: usize,
}

impl Layout {
    pub fn new(options: &CandidateOptions) -> Self {
        Self {
            labels: options.labels.clone(),
            first_listed: options.until_list.saturating_sub(1),
        }
    }

    /// 一覧から候補を選ぶキー。
    pub fn labels(&self) -> &[char] {
        &self.labels
    }

    /// 一ページの候補の数。
    pub fn page_size(&self) -> usize {
        self.labels.len()
    }

    /// 一覧に載る最初の候補の位置。
    ///
    /// **最初の数件は載らない。** そこは一つずつ見せる段階である。
    pub fn first_listed(&self) -> usize {
        self.first_listed
    }

    /// この位置の候補は一覧で見せる段階にあるか。
    pub fn listing(&self, index: usize) -> bool {
        index >= self.first_listed
    }

    /// この位置を含むページの先頭。一覧の前なら、その位置そのもの。
    pub fn page_start(&self, index: usize) -> usize {
        if index < self.first_listed {
            return index;
        }
        let size = self.page_size().max(1);
        self.first_listed + (index - self.first_listed) / size * size
    }
}
