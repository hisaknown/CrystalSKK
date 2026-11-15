//! 変換の候補を並べる言語モデル一式の導入 (ADR-0031)。
//!
//! 置き場所は辞書サーバの隣の `ranker` で、中身は二つに分かれる。
//!
//! - **llama.cpp の DLL** (`ranker\llama.cpp`)。公式のリリースから、版を
//!   固定した zip を取得し、ハッシュを確かめてから要るものだけ取り出す。
//!   こちらから再配布はしない。
//! - **モデルと語彙** (`model.gguf` と `tokenizer.json`)。かな漢字変換の
//!   モデル jinen を、Hugging Face から版 (コミット) を固定して取得する
//!   (ADR-0043)。こちらから再配布はしない。ハッシュをここに書いておき、
//!   違うものは置かない。`--ranker-from` で手元のフォルダを渡せば、そこから
//!   写す (取得できない環境向け)。
//!
//! どちらも辞書サーバが読み込むので、置くときにはサーバの exe と同じく、
//! 上書きできなければ古いものを退けてから置き、退けた残骸は次の導入で
//! 片付ける。
//!
//! **揃わなくても導入は止めない。** 並べ替えが効かないだけで、変換は
//! 辞書の順でできる。

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::report::Report;

/// llama.cpp の版。crystalskk-lm が構造体を合わせた版と同じでなければならない。
const LLAMA_CPP: &str = crystalskk_lm::LLAMA_VERSION;

/// 取得する zip と、そのハッシュ。辞書サーバと同じ種類の CPU のもの
/// (ADR-0037)。
#[cfg(not(target_arch = "aarch64"))]
const LLAMA_CPP_ZIP: &str = "llama-b11124-bin-win-cpu-x64.zip";
#[cfg(not(target_arch = "aarch64"))]
const LLAMA_CPP_SHA256: &str = "7eb4e7475f1730e0845e079e41f2e79b0c6de71d86731f755197129620a5bd28";
#[cfg(target_arch = "aarch64")]
const LLAMA_CPP_ZIP: &str = "llama-b11124-bin-win-cpu-arm64.zip";
#[cfg(target_arch = "aarch64")]
const LLAMA_CPP_SHA256: &str = "120f055384dd090094699ed5f4a6636755dbfc04bb1199ea7a0d9bc8f2a5ad6f";

/// zip から取り出すもの。x64 では CPU の実装が CPU ごとに分かれていて、
/// 動かす機械に合うものを llama.cpp が選ぶので、全部取り出す。ARM64 では
/// 一つ (`ggml-cpu.dll`) にまとまっている。
fn wanted(name: &str) -> bool {
    matches!(
        name,
        "llama.dll"
            | "ggml.dll"
            | "ggml-base.dll"
            | "ggml-cpu.dll"
            | "libomp.dll"
            | "LICENSE-LLVM-OpenMP"
    ) || (name.starts_with("ggml-cpu-") && name.ends_with(".dll"))
}

/// モデルの置き場所 (Hugging Face のリポジトリ) と、その版。版を上げる
/// ときは `MODEL_FILES` のハッシュと一緒に替える。
const MODEL_REPOSITORY: &str = "togatogah/jinen-v2-xsmall.gguf";
const MODEL_REVISION: &str = "3910fd01bf4ba86eca89617f18db9b0c1c5b2283";

/// モデル一式。(置く名前, 取得元での名前, ハッシュ)。
const MODEL_FILES: [(&str, &str, &str); 2] = [
    (
        crystalskk_server::paths::RANKER_MODEL,
        "jinen-v2-xsmall-Q8_0.gguf",
        "bc75a02512bdbb2a4786fec281a57d4d58ac3c2f118c3a29c2a9b41f2a4898b7",
    ),
    (
        crystalskk_server::paths::RANKER_TOKENIZER,
        "tokenizer.json",
        "477c91a788bf97e7ed993a4441cfb2c4d60033e505ade17d6c367bc09ed6e6ba",
    ),
];

/// 以前の一式 (llm-jp-3-150m) にだけあったファイル。導入のときに片付ける。
const RETIRED_FILES: [&str; 2] = ["LICENSE", "NOTICE.md"];

/// 版を書いておくファイル。同じ版なら取り直さない。
const VERSION_FILE: &str = "VERSION";

/// 取得して確かめた一式。置くのは [`Prepared::put`] で、辞書サーバを止めて
/// から行う。
///
/// **取得はサーバを止める前に済ませる。** 取得には数十秒かかることがあり、
/// そのあいだにどこかのアプリの TIP がサーバを起こし直すと、新しい一式を
/// 置き終える前のモデルを読まれてしまう。止めてから置くまでを短くする。
pub struct Prepared {
    /// llama.cpp の zip。いまのものと同じ版なら `None`。
    runtime: Result<Option<Vec<u8>>, String>,
    /// モデル一式のうち、置き換えるファイル。
    model: Result<Vec<(&'static str, Vec<u8>)>, String>,
    /// 置いたときに言う、モデルの出どころ。
    source: String,
}

/// 一式を取得して確かめる。何も置かない。`from` はモデル一式のあるフォルダ。
pub fn prepare(ranker_dir: &Path, from: Option<&Path>) -> Prepared {
    let runtime = fetch_runtime(&ranker_dir.join(crystalskk_server::paths::RANKER_RUNTIME));
    let (model, source) = match from {
        Some(from) => {
            let read = |name: &str| {
                let path = from.join(name);
                std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
            };
            (
                gather_model(read, ranker_dir),
                format!(
                    "言語モデルを置きました (元: {})。
",
                    from.display()
                ),
            )
        }
        None => (
            gather_model(download_model_file, ranker_dir),
            "言語モデル (jinen) を取得して置きました。
"
            .to_owned(),
        ),
    };
    Prepared {
        runtime,
        model,
        source,
    }
}

impl Prepared {
    /// 揃えた一式を置く。辞書サーバが止まっているうちに呼ぶ。
    ///
    /// **揃わなくても導入は止めない。** 並べ替えが効かないだけで、変換は
    /// 辞書の順でできる。
    pub fn put(self, ranker_dir: &Path, report: &Report) {
        report.say(
            "
",
        );
        let runtime_dir = ranker_dir.join(crystalskk_server::paths::RANKER_RUNTIME);
        match self.runtime.and_then(|zip| match zip {
            Some(zip) => place_runtime(&zip, &runtime_dir).map(|()| true),
            None => Ok(false),
        }) {
            Ok(true) => report.say(&format!(
                "llama.cpp {LLAMA_CPP} を取得して置きました。
"
            )),
            Ok(false) => {}
            Err(e) => report.say(&format!(
                "llama.cpp を置けません: {e}
"
            )),
        }
        for name in RETIRED_FILES {
            let _ = std::fs::remove_file(ranker_dir.join(name));
        }
        match self.model.and_then(|files| place_model(&files, ranker_dir)) {
            Ok(0) => {}
            Ok(_) => report.say(&self.source),
            Err(e) => {
                report.say(&format!(
                    "言語モデルを置けません: {e}
"
                ));
                report.say(
                    "候補の並べ替えは効きません。
",
                );
            }
        }
    }
}

/// llama.cpp の zip を取得して確かめる。いまのものと同じ版なら `None`。
fn fetch_runtime(directory: &Path) -> Result<Option<Vec<u8>>, String> {
    let version = directory.join(VERSION_FILE);
    if std::fs::read_to_string(&version).is_ok_and(|v| v.trim() == LLAMA_CPP) {
        return Ok(None);
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
    Ok(Some(body))
}

/// llama.cpp の DLL を置き、版を書いておく。
fn place_runtime(zip: &[u8], directory: &Path) -> Result<(), String> {
    extract(zip, directory)?;
    let version = directory.join(VERSION_FILE);
    std::fs::write(&version, LLAMA_CPP).map_err(|e| format!("{}: {e}", version.display()))
}

/// zip から要るものだけを取り出す。フォルダの構造は持ち込まない。
fn extract(zip: &[u8], directory: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    crate::install::sweep_retired_where(directory, wanted);
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
        crate::install::put(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// モデル一式のうち、置き換えるファイルを揃えて確かめる。`fetch` は置く
/// 名前から中身を得る。すでに同じものが置かれているファイルは取りに
/// 行かない。ハッシュが合わないものが一つでもあれば誤りにする。何も置かない。
fn gather_model(
    fetch: impl Fn(&str) -> Result<Vec<u8>, String>,
    ranker_dir: &Path,
) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    // 先に全部を揃えて確かめる。途中で違うものが見つかって、半端な一式が
    // 残るのを避ける。
    let mut fetched = Vec::new();
    for (name, _, expected) in MODEL_FILES {
        if file_matches(&ranker_dir.join(name), expected) {
            continue;
        }
        let bytes = fetch(name)?;
        let digest = sha256(&bytes);
        if digest != expected {
            return Err(format!(
                "{name} は、この版の CrystalSKK が知っているものではありません ({digest})"
            ));
        }
        fetched.push((name, bytes));
    }
    Ok(fetched)
}

/// 揃えたファイルを置く。置いた数を返す。
fn place_model(fetched: &[(&str, Vec<u8>)], ranker_dir: &Path) -> Result<usize, String> {
    if fetched.is_empty() {
        return Ok(0);
    }
    std::fs::create_dir_all(ranker_dir).map_err(|e| format!("{}: {e}", ranker_dir.display()))?;
    crate::install::sweep_retired_where(ranker_dir, |base| {
        MODEL_FILES.iter().any(|(name, _, _)| *name == base)
    });
    for (name, bytes) in fetched {
        let destination = ranker_dir.join(name);
        crate::install::put(&destination, bytes)
            .map_err(|e| format!("{}: {e}", destination.display()))?;
    }
    Ok(fetched.len())
}

/// モデル一式のファイルを一つ取得する。`name` は置く名前。
fn download_model_file(name: &str) -> Result<Vec<u8>, String> {
    let url = model_url(name).ok_or_else(|| format!("{name} の取得元がありません"))?;
    match crystalskk_fetch::get(&url, None).map_err(|e| format!("{url}: {e}"))? {
        crystalskk_fetch::Fetched::Downloaded(downloaded) => Ok(downloaded.body),
        crystalskk_fetch::Fetched::NotModified => Err(format!("{url}: 中身が返りません")),
    }
}

fn model_url(name: &str) -> Option<String> {
    let (_, remote, _) = MODEL_FILES.iter().find(|(local, _, _)| *local == name)?;
    Some(format!(
        "https://huggingface.co/{MODEL_REPOSITORY}/resolve/{MODEL_REVISION}/{remote}"
    ))
}

fn file_matches(path: &Path, expected: &str) -> bool {
    std::fs::read(path).is_ok_and(|bytes| sha256(&bytes) == expected)
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
        assert!(wanted("ggml-cpu.dll"));
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

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crystalskk-ranker-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_model_that_is_not_the_known_one_is_refused() {
        let from = scratch("from");
        for (name, _, _) in MODEL_FILES {
            std::fs::write(from.join(name), b"something else").unwrap();
        }
        let to = scratch("to");
        let read = |name: &str| std::fs::read(from.join(name)).map_err(|e| e.to_string());
        let error = gather_model(read, &to).unwrap_err();
        assert!(error.contains("知っているもの"), "{error}");
        assert!(!to.join("model.gguf").exists(), "一つも写さない");
    }

    #[test]
    fn the_model_is_taken_from_a_fixed_revision() {
        assert_eq!(
            model_url("model.gguf").as_deref(),
            Some(
                "https://huggingface.co/togatogah/jinen-v2-xsmall.gguf/resolve/3910fd01bf4ba86eca89617f18db9b0c1c5b2283/jinen-v2-xsmall-Q8_0.gguf"
            )
        );
        assert_eq!(model_url("LICENSE"), None);
    }
}
