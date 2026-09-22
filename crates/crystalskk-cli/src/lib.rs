//! CrystalSKK をターミナルから動かすための部品。
//!
//! 実行ファイルからも試験からも同じものを使えるよう、入力モードの実装は
//! ライブラリ側に置く。

pub mod interactive;
pub mod keys;
pub mod line;
pub mod render;
pub mod session;

pub use session::{Session, SessionBuilder};
