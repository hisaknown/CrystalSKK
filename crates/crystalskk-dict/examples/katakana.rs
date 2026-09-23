//! 辞書からカタカナ語の辞書を作って出す。
//! `cargo run -p crystalskk-dict --example katakana -- <辞書> > katakana.txt`
fn main() {
    let path = std::env::args().nth(1).expect("辞書を指定してください");
    let bytes = std::fs::read(&path).expect("辞書を読める");
    let decoded = crystalskk_dict::encoding::decode(&bytes);
    print!("{}", crystalskk_dict::derive::katakana_words(&decoded.text));
}
