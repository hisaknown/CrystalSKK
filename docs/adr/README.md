# Architecture Decision Records

CrystalSKK の設計判断を記録する。PRD (`../PRD.md`) が「何を作るか」を、ADR が「なぜその作り方にしたか」を持つ。

## 運用

- 1つの決定につき1ファイル。`NNNN-kebab-case-title.md`
- 番号は連番。欠番を作らない
- 一度 `Accepted` になった ADR は**書き換えない**。判断が変わったときは新しい ADR を書き、古い方を `Superseded by ADR-NNNN` に変更する
- 書くべきもの: 後から「なぜこうなっているのか」と問われうる決定。可逆で影響範囲の狭い判断は書かない

## ステータス

`Proposed` → `Accepted` / `Rejected` → （必要なら）`Superseded` / `Deprecated`

## 一覧

| # | タイトル | ステータス | 関連 |
|---|---|---|---|
| [0001](0001-separate-candidate-sources-from-ranking.md) | 候補の生成と並び替えを別のトレイトに分ける | Accepted | PRD §7 |
| [0002](0002-registration-as-a-stack-of-frames.md) | 辞書登録を第四の状態ではなく枠のスタックとして表現する | Accepted | PRD §5.1 |
