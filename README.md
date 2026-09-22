# CrystalSKK

Windows 向けの SKK 日本語入力メソッド。TSF の TIP として実装する。

**まだ動きません。** 変換エンジン (`crystalskk-core`) を作っている段階で、IME としては使えない。

## これは何か

[SKK](https://ja.wikipedia.org/wiki/SKK) 方式の日本語入力を Windows で行うためのソフトウェア。
先行実装として [SKK日本語入力FEP](http://coexe.web.fc2.com/skkfep.html)、[CorvusSKK](https://github.com/nathancorvussolis/corvusskk)、skkime がある。
CrystalSKK がそれらと違うところは次の3点。

- **設定を二層に分ける** — 大半の設定は宣言的な TOML で、それを超える調整はスクリプトで書く。どちらか一方に寄せない。
- **賢さを後から足せる構造** — 候補の生成と並び替えを最初から分離しておき、補完・予測変換・文脈を踏まえた提示を後付けではなく設計に織り込む。
- **Rust のみ** — C/C++ ツールチェインを要求しない。`cargo build` だけでビルドできる状態を維持する。

詳細は [PRD](docs/PRD.md) を参照。設計判断の経緯は [ADR](docs/adr/) に置く。

## 構成

| クレート | 役割 | 状態 |
|---|---|---|
| `crystalskk-core` | 変換状態機械、ローマ字変換、候補処理。OS 非依存・I/O なし | 着手 |
| `crystalskk-dict` | 辞書の読み書きと検索、ユーザー辞書の永続化 | 着手 |
| `crystalskk-fetch` | 辞書の取得と設置 (WinHTTP) | 着手 |
| `crystalskk-cli` | ターミナルから変換を動かす確認用ツール | 着手 |
| `crystalskk-server` | 変換サーバー。IPC、設定、スクリプト実行 | 未着手 |
| `crystalskk-tip` | TSF TIP (cdylib) | 着手 |
| `crystalskk-setup` | この環境への導入と削除 | 着手 |
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

`-i` を付けると、一打鍵ずつ受け取る対話モードになる。実際の打鍵の手触りはこちらで確かめる。Ctrl+C で終了。

```bash
cargo run -p crystalskk-cli -- -i --dict ./SKK-JISYO.L
```

### IME を導入する

まだ入力はできない。入力方式として選べるところまで。

```bash
cargo build -p crystalskk-tip --release
```

```bash
cargo run -p crystalskk-setup -- install
```

DLL は `%ProgramFiles%\CrystalSKK\bin` に写してから登録される。`target` の中身を直接登録しないのは、使用中の DLL がビルドに掴まれて作り直せなくなるのを避けるため。

```bash
cargo run -p crystalskk-setup -- status
```

```bash
cargo run -p crystalskk-setup -- uninstall
```

入力方式の登録は機械全体に書かれるため、管理者権限が要る ([ADR-0007](docs/adr/0007-installing-a-tip-requires-administrator.md))。権限がなければ UAC の確認が出るので、応じればよい。導入後、設定 → 言語と地域 → 日本語 → 言語のオプション → キーボード に CrystalSKK が現れる。

登録される DLL はビルド成果物とは別物なので、IME を有効にしたままでも `cargo build` は通る。入れ替えるときだけ `install` をやり直す。ただし DLL を読み込んでいるアプリは先に閉じること。

### TIP の様子を見る

TIP は他人のプロセスの中で動くので、標準出力もデバッガも当てにできない。環境変数を設定したアプリから使うと、`%LOCALAPPDATA%\CrystalSKK\tip.log` に記録が残る。

```bash
setx CRYSTALSKK_LOG 1
```

設定したあとに起動したアプリから記録される。サインインし直すと確実。有効化されたか、打鍵が届いているかを切り分けるのに使う。**入力のたびにファイルを開くので、常用しないこと。**

記録は UTF-8 なので、Windows PowerShell で読むときは符号化を指定する。

```bash
Get-Content -Encoding UTF8 $env:LOCALAPPDATA\CrystalSKK\tip.log -Tail 40
```

依存に C をビルドするクレート (`cc`) を入れないことを CI で検査している。新しい依存を足すときは `cargo tree --invert cc` が空であることを確認すること。

## ライセンス

MIT License ([LICENSE](LICENSE))

辞書は同梱しない。SKK 辞書はそれぞれのライセンス (SKK-JISYO.L は GPL 系) に従う。
