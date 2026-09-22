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
| — | まだ記録なし | | |

<!-- 例:
| [0001](0001-tsf-tip-architecture.md) | TSF TIP として実装する | Accepted | PRD §7 |
-->
