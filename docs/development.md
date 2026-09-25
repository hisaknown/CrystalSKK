# 開発

CrystalSKK を手元でビルドし、試し、導入するための手順。用語は [CONTEXT.md](../CONTEXT.md)、要求は [PRD](PRD.md)、設計判断の経緯は [ADR](adr/) にある。

## 構成

| クレート | 役割 | 状態 |
|---|---|---|
| `crystalskk-core` | 変換状態機械、ローマ字変換、候補処理。OS 非依存・I/O なし | 着手 |
| `crystalskk-dict` | 辞書の読み書きと検索、ユーザー辞書の永続化 | 着手 |
| `crystalskk-fetch` | 辞書の取得と設置 (WinHTTP) | 着手 |
| `crystalskk-cli` | ターミナルから変換を動かす確認用ツール | 着手 |
| `crystalskk-ipc` | TIP と辞書サーバが交わす語彙 | 着手 |
| `crystalskk-server` | 辞書サーバー。辞書と設定ファイルを持つ唯一のプロセス | 着手 |
| `crystalskk-settings` | 設定ファイルの読み込みと、足りない項目の書き足し | 着手 |
| `crystalskk-tip` | TSF TIP (cdylib) | 着手 |
| `crystalskk-setup` | この環境への導入と削除 | 着手 |
| `crystalskk-art` | アイコンの SVG を描いて埋め込む (ビルドのときだけ使う) | 着手 |
| `crystalskk-config` | 設定 GUI | 未着手 |

## 開発

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

辞書を取得して手元に置く:

```bash
cargo run -p crystalskk-fetch --example install-dict -- ./SKK-JISYO.L
```

ターミナルで変換を試す:

```bash
cargo run -p crystalskk-cli -- --dict ./SKK-JISYO.L
```

既定は行入力モードで、一行が打鍵列になる。英大文字がシフト付きの打鍵、空白が Space、`\n` が Enter。起動後に `:help` で表記の一覧が出る。

```text
> Kanji\s
  文書:   (なし)
  未確定: ▼漢字
  候補:   [漢字]   幹事    監事    感じ
  モード: あ ひらがな
```

CLI も IME と同じ設定ファイルの形を使う。既定では今いる場所の `crystalskk-config.toml` と `romaji.txt` を使い、無ければ雛形から作る (場所は `--config` で変えられる)。

`-i` を付けると、一打鍵ずつ受け取る対話モードになる。実際の打鍵の手触りはこちらで確かめる。Ctrl+C で終了。

```bash
cargo run -p crystalskk-cli -- -i --dict ./SKK-JISYO.L
```

### IME を導入する

TIP (DLL) と辞書サーバをビルドしてから導入する。導入は `target/release` にあるものを写して登録し、辞書サーバを起こす。

```bash
cargo build --workspace --release
```

```bash
cargo run -p crystalskk-setup --release -- install
```

変換の候補を前後の文章から並べる言語モデル ([ADR-0030](adr/0030-rank-candidates-with-a-small-language-model-in-the-server.md)) も、導入のときに一緒に置かれる。モデル一式は `tools/ranker-model` で作る ([uv](https://docs.astral.sh/uv/) が要る)。作り直しても同じものになり、導入はハッシュを確かめてから `target/ranker-model/out` のものを写す。llama.cpp の DLL は導入のときに公式のリリースから取得する ([ADR-0031](adr/0031-ship-the-ranker-model-with-the-program.md))。どちらも揃わなければ、並べ替えが効かないだけで変換はできる。使うかどうかは設定ファイルの `[ranker]` で決める。

```bash
cd tools/ranker-model && uv run build.py
```

使う辞書は設定ファイルの `dictionaries.sources` に並べる (既定は L 辞書)。URL なら辞書サーバが裏で取得し、起動のたびに更新を確かめる。複数並べると、一つの辞書であるかのように並べた順で引く ([ADR-0022](adr/0022-read-the-listed-dictionaries-as-one.md))。いま取り直したいとき、何が起きたかを見たいときは次を打つ。権限は要らない。

```bash
cargo run -p crystalskk-setup -- dict
```

DLL は `%ProgramFiles%\CrystalSKK\bin` に、辞書は `%LOCALAPPDATA%\CrystalSKK\dictionaries` に置かれる。辞書は利用者のデータであってプログラムの一部ではないので、場所を分けている。DLL を `target` から直接登録しないのは、使用中の DLL がビルドに掴まれて作り直せなくなるのを避けるため。

```bash
cargo run -p crystalskk-setup -- status
```

```bash
cargo run -p crystalskk-setup -- uninstall
```

入力方式の登録は機械全体に書かれるため、管理者権限が要る ([ADR-0007](adr/0007-installing-a-tip-requires-administrator.md))。権限がなければ UAC の確認が出るので、応じればよい。導入後、設定 → 言語と地域 → 日本語 → 言語のオプション → キーボード に CrystalSKK が現れる。

登録される DLL はビルド成果物とは別物なので、IME を有効にしたままでも `cargo build` は通る。入れ替えるときは `install` をやり直すだけでよい。使用中の DLL は別名へ退けられるため、アプリを閉じる必要もサインインし直す必要もない。ただし**すでに開いているアプリは古い DLL を使い続ける**ので、入れ替えを試すときはそのアプリを開き直すこと。

### TIP の様子を見る

TIP は他人のプロセスの中で動くので、標準出力もデバッガも当てにできない。代わりに `%LOCALAPPDATA%\CrystalSKK\tip.log` に記録を残す。既定は `info` (有効化や設定の読み込みなどの節目だけ) で、大きくなりすぎたら一代だけ退けて書き直す。

細かさは導入先の目印ファイルで決める。管理者権限が要る。

```bash
cargo run -p crystalskk-setup -- log trace
```

段階は `off` / `error` (失敗だけ) / `info` / `trace` (打鍵ごと)。変えたあとに起動したアプリから効く。**`trace` は入力のたびにファイルを開くので、確かめ終えたら `info` か `off` に戻すこと。** 環境変数 `CRYSTALSKK_LOG` でも同じ段階を指定できる (詳しいほうが採られる) が、ストアアプリには環境変数が届かない。

記録は UTF-8 なので、Windows PowerShell で読むときは符号化を指定する。

```bash
Get-Content -Encoding UTF8 $env:LOCALAPPDATA\CrystalSKK\tip.log -Tail 40
```

### アイコン

アイコンの正本は `assets/icons/` の SVG で、TIP をビルドするときに各大きさに描いて埋め込む ([ADR-0024](adr/0024-draw-the-icons-from-svg-at-build-time.md))。入力モードの絵は黒一色で描けばよく、色はタスクバーの明るさに合わせて動くときに付く。文字はアウトライン化しておくこと (`<text>` が残っているとビルドが止まる)。明るい地と暗い地に並べた見本は次で作れる。

```bash
cargo run -p crystalskk-art --example sheet -- sheet.png assets/icons/mode-hiragana.svg assets/icons/face.svg
```

### C を持ち込まない

依存に C をビルドするクレート (`cc`) を入れないことを CI で検査している。新しい依存を足すときは `cargo tree --invert cc` が空であることを確認すること。

