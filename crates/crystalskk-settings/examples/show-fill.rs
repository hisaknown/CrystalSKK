//! 書き足した結果を見る。`cargo run -p crystalskk-settings --example show-fill -- <file>`
fn main() {
    let path = std::env::args().nth(1).expect("ファイルを指定してください");
    match crystalskk_settings::load(std::path::Path::new(&path)) {
        Ok(loaded) => {
            print!("{}", loaded.text);
            eprintln!(
                "--- 書き足した: {:?} / 知らない: {:?}",
                loaded.added, loaded.unknown
            );
        }
        Err(e) => eprintln!("誤り: {e}"),
    }
}
