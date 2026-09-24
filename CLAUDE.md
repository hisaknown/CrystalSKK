# CLAUDE.md

## 文字コードと改行

- すべてのテキストファイルは **UTF-8 (BOM なし)・LF** で書く。`.gitattributes` (`* text=auto eol=lf`) と `.editorconfig` で固定している。
- ファイルの作成・編集には **Edit / Write ツールと Bash ツールだけ**を使う。PowerShell でファイルを書かない (`Set-Content` や `>` は既定で CRLF・BOM 付き・ANSI などになり、文字化けや改行混在の原因になる)。
- Bash で書くときも CRLF を持ち込まない。混ざったら `sed -i 's/\r$//' <file>` で直す。
- 確認は `git ls-files --eol | grep crlf` (何も出なければよい)。
- Windows 側のツール (`reg`、`regsvr32`、`wevtutil`、`cmd` の組み込みコマンドなど) の出力は **cp932** で、そのまま読むと文字化けする。化けていたら `| iconv -f cp932 -t utf-8` を通して読む。化けた出力をそのままファイルやコードに貼らない。
