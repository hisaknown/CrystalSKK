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
| [0003](0003-utf8-internally-convert-on-load.md) | 内部表現とユーザー辞書を UTF-8 に統一し、EUC-JP は読み込み時に変換する | Accepted | PRD Q-07 |
| [0004](0004-fetch-dictionaries-over-the-system-http-stack.md) | 辞書の取得に OS の HTTP スタック (WinHTTP) を使う | Accepted | PRD N-08, F-10 |
| [0005](0005-rescue-only-impossible-input.md) | 打ち間違いの救済は、入力が矛盾するときだけ行う | Accepted | PRD §3 |
| [0006](0006-register-the-tip-per-user-during-development.md) | 開発中の TIP は利用者ごとに登録する | Superseded by 0007 | PRD §10 |
| [0007](0007-installing-a-tip-requires-administrator.md) | TIP の導入には管理者権限が要る | Accepted | ADR-0006 |
| [0008](0008-write-to-the-document-only-through-a-composition.md) | 文書への書き込みは必ず composition を通す | Accepted | PRD §3 |
| [0009](0009-ship-the-profile-icon-as-an-ico-file.md) | 設定画面に出す絵は `.ico` ファイルとして置く | Accepted | PRD N-08 |
| [0010](0010-publish-the-input-mode-through-compartments.md) | 入力モードは区画に書いて伝える | Accepted (一部を 0012 が覆す) | ADR-0009 |
| [0011](0011-declare-every-capability-as-a-category.md) | 使える場面は分類として名乗る | Accepted | ADR-0010 |
| [0012](0012-the-open-close-compartment-is-the-truth.md) | 入力方式の入切は向こうが決める | Accepted | ADR-0010 |
| [0013](0013-on-is-the-resting-state.md) | 入が常態で、切は SKK に手を引かせるためにある | Accepted | ADR-0012 |
| [0014](0014-let-app-containers-read-the-dictionary.md) | 隔離されたアプリには辞書を読ませる。書かせはしない | Superseded by 0016 | ADR-0011, PRD §7 |
| [0015](0015-draw-the-candidate-list-and-offer-it-too.md) | 候補一覧は自前で描き、同時に差し出す | Accepted | PRD Q-02 |
| [0016](0016-the-dictionary-lives-in-one-process.md) | 辞書は一つのプロセスだけが持つ | Accepted | ADR-0001, ADR-0014, PRD Q-04 |
| [0017](0017-draw-the-underlines-ourselves.md) | 下線は自分で引き、色は決めない | Accepted | ADR-0011, PRD Q-09 |
| [0018](0018-keep-the-markers-out-of-the-document.md) | 印は文書に出さない | Accepted | ADR-0017, PRD Q-09 |
| [0019](0019-complete-the-heading-while-it-is-typed.md) | 打っている最中に見出し語の続きを補完する | Accepted | ADR-0001, ADR-0016, ADR-0017 |
| [0020](0020-the-settings-file-is-the-whole-truth.md) | 設定ファイルに書かれていることを、効いている設定のすべてにする | Accepted | PRD F-14, ADR-0016, ADR-0019 |
| [0021](0021-the-romaji-table-belongs-to-the-user.md) | ローマ字テーブルは利用者の持ち物として別のファイルに置き、書き換えない | Accepted | ADR-0020, PRD F-14, PRD §5.3 |
