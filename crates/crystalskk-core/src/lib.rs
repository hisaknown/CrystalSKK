//! CrystalSKK の変換エンジン。
//!
//! このクレートは OS に依存せず、I/O も行わない。キー入力を受け取って
//! 「確定した文字列」と「未確定の表示状態」を返す純粋な状態機械であり、
//! TSF TIP からもテストからも同じように駆動できる。
//!
//! 辞書はトレイト経由で外から与える。候補の生成 (`CandidateSource`) と
//! 並び替え (`Ranker`) は分離しておき、将来の補完・予測変換を後から
//! 差し込めるようにする。

pub mod dict;
pub mod engine;
pub mod kana;
pub mod key;
pub mod mode;
pub mod options;
pub mod romaji;

pub use dict::{Candidate, CandidateSource, Context, Query, Ranker};
pub use engine::{Engine, Event, Marker, Preedit, Response};
pub use key::Key;
pub use mode::InputMode;
pub use options::Options;
pub use romaji::{RomajiConverter, RomajiTable, Rule};
