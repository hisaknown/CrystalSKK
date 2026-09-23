"""変換の候補を並べる言語モデルを、配る形に作る (ADR-0031)。

    uv run build.py

作るのは `target/ranker-model/out/` の一式で、これをそのまま公開する。

- `model.gguf`      llm-jp-3-150m を GGUF にし、Q4_0 に量子化したもの
- `tokenizer.json`  その語彙。トークン化は CrystalSKK が Rust の tokenizers で行う
- `LICENSE`         Apache License 2.0 の本文
- `NOTICE.md`       元のモデルと、手を加えた点
- `SHA256SUMS`      上の四つのハッシュ

**利用者の手元では動かさない。** 変換には Python と torch と llama.cpp の
ソースが要る。作るのは開発者で、利用者は出来上がったものを取得する。

# 何を固定しているか

元のモデルの版 (`MODEL_REVISION`) と llama.cpp の版 (`LLAMA_CPP`) を固定する。
llama.cpp の版は crystalskk-lm が構造体を合わせた版と同じにする。

# 変換で手を入れているところ

llama.cpp の変換スクリプトは llm-jp の語彙の前処理を知らず、止まる。
CrystalSKK は llama.cpp のトークナイザーを使わないので、**ファイルは
書き換えず、ここで該当の関数だけを差し替えてから変換を呼ぶ。**

その語彙 (結合規則の無い BPE) のままでは llama.cpp がモデルを読めない。
そこで最後に、トークン一覧は残したまま、結合規則の要らない SentencePiece
形式として書き直す。
"""

import hashlib
import io
import runpy
import shutil
import subprocess
import sys
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "target" / "ranker-model"
OUT = WORK / "out"

MODEL = "llm-jp/llm-jp-3-150m"
MODEL_REVISION = "b112feef602fff752e4dac4c30af6a2c2fa41c7a"
LLAMA_CPP = "b11124"
LLAMA_CPP_REPO = "https://github.com/ggml-org/llama.cpp"
APACHE_LICENSE = "https://www.apache.org/licenses/LICENSE-2.0.txt"


def step(message):
    print(f"==> {message}", flush=True)


def llama_cpp_source():
    """llama.cpp のソース。変換スクリプトと gguf-py を使う。"""
    source = WORK / f"llama.cpp-{LLAMA_CPP}"
    if not source.exists():
        step(f"llama.cpp {LLAMA_CPP} のソースを取得する")
        subprocess.run(
            ["git", "clone", "--quiet", "--depth", "1", "--branch", LLAMA_CPP, LLAMA_CPP_REPO, str(source)],
            check=True,
        )
    return source


def llama_quantize():
    """ビルド済みの llama-quantize。C++ はビルドしない。"""
    folder = WORK / f"llama-{LLAMA_CPP}-bin-win-cpu-x64"
    exe = folder / "llama-quantize.exe"
    if not exe.exists():
        name = f"llama-{LLAMA_CPP}-bin-win-cpu-x64.zip"
        step(f"{name} を取得する")
        with urllib.request.urlopen(f"{LLAMA_CPP_REPO}/releases/download/{LLAMA_CPP}/{name}") as response:
            zipfile.ZipFile(io.BytesIO(response.read())).extractall(folder)
    return exe


def model_snapshot():
    from huggingface_hub import snapshot_download

    step(f"{MODEL}@{MODEL_REVISION[:8]} を取得する")
    return Path(snapshot_download(MODEL, revision=MODEL_REVISION))


def convert(source, snapshot, f16):
    """GGUF (f16) にする。語彙の前処理の判定だけ差し替える。"""
    step("GGUF (f16) に変換する")
    sys.path[:0] = [str(source), str(source / "gguf-py")]
    from conversion.base import TextModel

    TextModel.get_vocab_base_pre = lambda self, tokenizer: "default"
    argv = sys.argv
    sys.argv = ["convert_hf_to_gguf.py", str(snapshot), "--outtype", "f16", "--outfile", str(f16)]
    try:
        runpy.run_path(str(source / "convert_hf_to_gguf.py"), run_name="__main__")
    finally:
        sys.argv = argv


def quantize(exe, f16, q4):
    """Q4_0 にする。埋め込みと出力層も Q4_0 にする (出力層は採点のたびに通る)。"""
    step("Q4_0 に量子化する")
    subprocess.run(
        [str(exe), "--output-tensor-type", "q4_0", "--token-embedding-type", "q4_0", str(f16), str(q4), "Q4_0"],
        check=True,
        stdout=subprocess.DEVNULL,
    )


def rewrite_vocab(q4, out):
    """語彙を、結合規則の要らない SentencePiece 形式として書き直す。"""
    step("語彙の形式を書き直す")
    import gguf
    from gguf.scripts.gguf_new_metadata import MetadataDetails, copy_with_new_metadata

    reader = gguf.GGUFReader(str(q4), "r")
    n = len(reader.fields["tokenizer.ggml.tokens"].data)
    arch = reader.fields["general.architecture"]
    writer = gguf.GGUFWriter(str(out), arch.parts[arch.data[0]].tobytes().decode())
    new = {
        "tokenizer.ggml.model": MetadataDetails(gguf.GGUFValueType.STRING, "llama"),
        "tokenizer.ggml.scores": MetadataDetails(
            gguf.GGUFValueType.ARRAY, [0.0] * n, sub_type=gguf.GGUFValueType.FLOAT32
        ),
    }
    if "tokenizer.ggml.token_type" not in reader.fields:
        new["tokenizer.ggml.token_type"] = MetadataDetails(
            gguf.GGUFValueType.ARRAY, [1] * n, sub_type=gguf.GGUFValueType.INT32
        )
    remove = [k for k in ("tokenizer.ggml.merges", "tokenizer.ggml.pre") if k in reader.fields]
    copy_with_new_metadata(reader, writer, new, remove)


NOTICE = f"""# CrystalSKK の候補並べ替え用モデル

このフォルダのモデルと語彙は、次のモデルから作ったものです。

- 元のモデル: {MODEL} (https://huggingface.co/{MODEL})
- 版: {MODEL_REVISION}
- 使用許諾: Apache License 2.0 (同梱の LICENSE)

## 手を加えた点

- llama.cpp {LLAMA_CPP} の変換スクリプトで GGUF 形式にしました。
- Q4_0 に量子化しました (埋め込みと出力層を含む)。
- GGUF に書かれた語彙の形式を SentencePiece として書き直しました。
  トークンの一覧と番号は変えていません。CrystalSKK はトークン化に
  tokenizer.json を使い、GGUF の語彙は使いません。

tokenizer.json は元のモデルのものを、そのまま含めています。
"""


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    source = llama_cpp_source()
    exe = llama_quantize()
    snapshot = model_snapshot()

    f16 = WORK / "model-f16.gguf"
    q4 = WORK / "model-q4_0.gguf"
    convert(source, snapshot, f16)
    quantize(exe, f16, q4)

    if OUT.exists():
        shutil.rmtree(OUT)
    OUT.mkdir(parents=True)
    rewrite_vocab(q4, OUT / "model.gguf")
    shutil.copy(snapshot / "tokenizer.json", OUT / "tokenizer.json")
    step("使用許諾を添える")
    with urllib.request.urlopen(APACHE_LICENSE) as response:
        (OUT / "LICENSE").write_bytes(response.read())
    (OUT / "NOTICE.md").write_text(NOTICE, encoding="utf-8", newline="\n")

    lines = []
    for name in ("model.gguf", "tokenizer.json", "LICENSE", "NOTICE.md"):
        digest = hashlib.sha256((OUT / name).read_bytes()).hexdigest()
        lines.append(f"{digest}  {name}\n")
    (OUT / "SHA256SUMS").write_text("".join(lines), encoding="utf-8", newline="\n")
    print("".join(lines), end="")
    step(f"できました: {OUT}")


if __name__ == "__main__":
    main()
