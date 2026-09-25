# CrystalSKK

Windows 向けの SKK 日本語入力メソッド。TSF の TIP として実装する。

**日々の入力に要る機能がひととおり揃った。** かな入力と変換、候補ウィンドウ (ストアアプリにも出る)、辞書登録、学習、下線による区切りの表示、動的補完、切り替えたときにカーソルのそばへ出る入力モードの表示が動く。辞書は設定に並べたものを自動で取得し、設定はすべて一つのファイルに書かれる。

まだ無いもの: 配布用のインストーラ (いまはソースからビルドして導入する)、設定 GUI、設定スクリプト、キーバインドの設定。

## これは何か

[SKK](https://ja.wikipedia.org/wiki/SKK) 方式の日本語入力を Windows で行うためのソフトウェア。
先行実装として [SKK日本語入力FEP](http://coexe.web.fc2.com/skkfep.html)、[CorvusSKK](https://github.com/nathancorvussolis/corvusskk)、skkime がある。
CrystalSKK がそれらと違うところは次の3点。

- **設定を二層に分ける** — 大半の設定は宣言的な TOML で、それを超える調整はスクリプトで書く。どちらか一方に寄せない。
- **賢さを後から足せる構造** — 候補の生成と並び替えを最初から分離しておき、補完・予測変換・文脈を踏まえた提示を後付けではなく設計に織り込む。
- **Rust のみ** — C/C++ ツールチェインを要求しない。`cargo build` だけでビルドできる状態を維持する。

詳細は [PRD](docs/PRD.md) を参照。設計判断の経緯は [ADR](docs/adr/) に置く。

## 設定

設定ファイルは `%LOCALAPPDATA%\CrystalSKK\config.toml`。導入のときに雛形から作られ、全項目が説明付きで書かれている。

**このファイルに書かれている値が、効いている設定のすべてである** ([ADR-0020](docs/adr/0020-the-settings-file-is-the-whole-truth.md))。CrystalSKK は既定値を持たない。新しい版で項目が増えたときは、導入のときにそのファイルへ書き足され、何を足したかが表示される。書き換えた値は、入力先を切り替えたときに効く。

ローマ字テーブルは隣の `romaji.txt` (Google 日本語入力と同じタブ区切り 3 列)。**利用者の持ち物**として扱い、無いときに作るだけで、版を上げても書き換えない ([ADR-0021](docs/adr/0021-the-romaji-table-belongs-to-the-user.md))。

候補の窓とカーソルのそばの窓の色は `[colors]` で決める。明るい組と暗い組を書いておき、アプリの明るさに合わせて選ぶ ([ADR-0026](docs/adr/0026-colours-of-the-popups-are-settings.md))。

トレイの入力モード表示を右クリックすると、設定フォルダを開く・設定を読み直す・設定ファイルやローマ字テーブルを雛形で上書きする (元の中身は `.bak` に退避) ことができる。

## 開発

ソースからのビルドと導入、ログの見方などは [docs/development.md](docs/development.md) を参照。

## ライセンス

MIT License ([LICENSE](LICENSE))

辞書は同梱しない。SKK 辞書はそれぞれのライセンス (SKK-JISYO.L は GPL 系) に従う。
