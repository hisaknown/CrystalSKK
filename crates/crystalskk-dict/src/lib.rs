//! SKK 辞書の読み書きと検索。
//!
//! 静的辞書とユーザー辞書は、どちらも同じ SKK 標準のテキスト形式を扱う。
//! 内部表現とユーザー辞書の保存は UTF-8 に統一し、EUC-JP の辞書は読み込む
//! ときに一度だけ変換する (ADR-0003)。
//!
//! 辞書はネットワークやファイルから来る外部データなので、壊れていても
//! 入力自体は続けられなければならない (PRD N-09)。解釈できない行は
//! 読み飛ばし、件数だけ [`LoadReport`] で報告する。

pub mod encoding;
pub mod format;
pub mod memory;
pub mod user;

pub use encoding::{Decoded, decode};
pub use memory::{LoadReport, MemoryDict};
pub use user::UserDict;
