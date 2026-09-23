//! 変換の候補を並べる言語モデル一式の導入 (ADR-0031)。
//!
//! 置き場所は辞書サーバの隣の `ranker` で、中身は二つに分かれる。
//!
//! - **llama.cpp の DLL** (`ranker\llama.cpp`)。公式のリリースから、版を
//!   固定した zip を取得し、ハッシュを確かめてから要るものだけ取り出す。
//!   こちらから再配布はしない。
//! - **モデルと語彙** (`model.gguf` ほか)。`tools/ranker-model/build.py` で
//!   作ったものを写す。作り直しても同じバイト列になるので、ハッシュを
//!   ここに書いておき、違うものは置かない。
//!
//! **揃わなくても導入は止めない。** 並べ替えが効かないだけで、変換は
//! 辞書の順でできる。

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::report::Report;

/// llama.cpp の版。crystalskk-lm が構造体を合わせた版と同じでなければならない。
const LLAMA_CPP: &str = crystalskk_lm::LLAMA_VERSION;

/// 取得する zip と、そのハッシュ。
const LLAMA_CPP_ZIP: &str = "llama-b11124-bin-win-cpu-x64.zip";
const LLAMA_CPP_SHA256: &str = "7eb4e7475f1730e0845e079e41f2e79b0c6de71d86731f755197129620a5bd28";

/// zip から取り出すもの。CPU の実装は CPU ごとに分かれていて、動かす
/// 機械に合うものを llama.cpp が選ぶので、全部取り出す。
fn wanted(name: &str) -> bool {
    matches!(
        name,
        "llama.dll" | "ggml.dll" | "ggml-base.dll" | "libomp.dll" | "LICENSE-LLVM-OpenMP"
    ) || (name.starts_with("ggml-cpu-") && name.ends_with(".dll"))
}

/// モデル一式と、そのハッシュ (`build.py` の `SHA256SUMS` と同じ)。
const MODEL_FILES: [(&str, &str); 4] = [
    (
        "model.gguf",
        "a9cb8ae9e80ea2688009049ed01ee9b10ef1c1839a8d78d1e4680bb00ba59f06",
    ),
    (
        "tokenizer.json",
        "955dc1fa623fab38cc92a3f4ee172423ae6d73201c4207569bfdf5626bc733f0",
    ),
    (
        "LICENSE",
        "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
    ),
    (
        "NOTICE.md",
        "0e23d469c2a9ee74d96e5b7b7e640298056c34275fe42af3817736643966fe06",
    ),
];

/// 版を書いておくファイル。同じ版なら取り直さない。
const VERSION_FILE: &str = "VERSION";

/// 一式を揃える。`from` はモデル一式のあるフォルダ。
pub fn install(ranker_dir: &Path, from: Option<&Path>, report: &Report) {
    report.say("\n");
    match install_runtime(&ranker_dir.join(crystalskk_server::paths::RANKER_RUNTIME)) {
        Ok(true) => report.say(&format!("llama.cpp {LLAMA_CPP} を取得して置きました。\n")),
        Ok(false) => {}
        Err(e) => report.say(&format!("llama.cpp を置けません: {e}\n")),
    }
    match from.map(Path::to_path_buf).or_else(default_model_source) {
        Some(from) => match install_model(&from, ranker_dir) {
            Ok(0) => {}
            Ok(_) => report.say(&format!(
                "言語モデルを置きました (元: {})。\n",
                from.display()
            )),
            Err(e) => report.say(&format!("言語モデルを置けません: {e}\n")),
        },
        None if model_is_in_place(ranker_dir) => {}
        None => {
            report.say("言語モデルが見つかりません。候補の並べ替えは効きません。\n");
            report.say("tools/ranker-model で uv run build.py を実行してください。\n");
        }
    }
}

/// llama.cpp の DLL を揃える。取得したら `true`。
fn install_runtime(directory: &Path) -> Result<bool, String> {
    let version = directory.join(VERSION_FILE);
    if std::fs::read_to_string(&version).is_ok_and(|v| v.trim() == LLAMA_CPP) {
        return Ok(false);
    }
    let url = format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_CPP}/{LLAMA_CPP_ZIP}"
    );
    let body = match crystalskk_fetch::get(&url, None).map_err(|e| format!("{url}: {e}"))? {
        crystalskk_fetch::Fetched::Downloaded(downloaded) => downloaded.body,
        crystalskk_fetch::Fetched::NotModified => return Err(format!("{url}: 中身が返りません")),
    };
    let digest = sha256(&body);
    if digest != LLAMA_CPP_SHA256 {
        return Err(format!("{LLAMA_CPP_ZIP} のハッシュが合いません ({digest})"));
    }
    extract(&body, directory)?;
    std::fs::write(&version, LLAMA_CPP).map_err(|e| format!("{}: {e}", version.display()))?;
    Ok(true)
}

/// zip から要るものだけを取り出す。フォルダの構造は持ち込まない。
fn extract(zip: &[u8], directory: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = entry
            .enclosed_name()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        else {
            continue;
        };
        if !wanted(&name) {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{name}: {e}"))?;
        let path = directory.join(&name);
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// モデル一式を写す。ハッシュが合わないものは写さない。写した数を返す。
fn install_model(from: &Path, ranker_dir: &Path) -> Result<usize, String> {
    // 先に全部を確かめる。途中で違うものが見つかって、半端な一式が
    // 残るのを避ける。
    for (name, expected) in MODEL_FILES {
        let path = from.join(name);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let digest = sha256(&bytes);
        if digest != expected {
            return Err(format!(
                "{} は、この版の CrystalSKK が知っているものではありません ({digest})",
                path.display()
            ));
        }
    }
    std::fs::create_dir_all(ranker_dir).map_err(|e| format!("{}: {e}", ranker_dir.display()))?;
    let mut copied = 0;
    for (name, expected) in MODEL_FILES {
        let destination = ranker_dir.join(name);
        if file_matches(&destination, expected) {
            continue;
        }
        std::fs::copy(from.join(name), &destination)
            .map_err(|e| format!("{}: {e}", destination.display()))?;
        copied += 1;
    }
    Ok(copied)
}

/// モデル一式が、この版のものとして揃っているか。
fn model_is_in_place(ranker_dir: &Path) -> bool {
    MODEL_FILES
        .iter()
        .all(|(name, expected)| file_matches(&ranker_dir.join(name), expected))
}

fn file_matches(path: &Path, expected: &str) -> bool {
    std::fs::read(path).is_ok_and(|bytes| sha256(&bytes) == expected)
}

/// `build.py` の出力。ビルド成果物と同じく、手元から探す。
fn default_model_source() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("target/ranker-model/out")];
    if let Ok(exe) = std::env::current_exe()
        && let Some(target) = exe.parent().and_then(Path::parent)
    {
        candidates.push(target.join("ranker-model").join("out"));
    }
    candidates
        .into_iter()
        .find(|dir| dir.join(MODEL_FILES[0].0).is_file())
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_llama_needs_is_taken_from_the_zip() {
        assert!(wanted("llama.dll"));
        assert!(wanted("ggml-cpu-zen4.dll"));
        assert!(wanted("LICENSE-LLVM-OpenMP"));
        assert!(!wanted("llama-cli.exe"));
        assert!(!wanted("ggml-rpc.dll"));
        assert!(!wanted("mtmd.dll"));
    }

    #[test]
    fn the_hash_is_written_in_lowercase_hex() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crystalskk-ranker-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_model_that_is_not_the_known_one_is_refused() {
        let from = scratch("from");
        for (name, _) in MODEL_FILES {
            std::fs::write(from.join(name), b"something else").unwrap();
        }
        let to = scratch("to");
        let error = install_model(&from, &to).unwrap_err();
        assert!(error.contains("知っているもの"), "{error}");
        assert!(!to.join("model.gguf").exists(), "一つも写さない");
    }
}
