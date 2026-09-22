# ADR-0006: 開発中の TIP は利用者ごとに登録する

- ステータス: Superseded by [ADR-0007](0007-installing-a-tip-requires-administrator.md)
- 日付: 2026-09-22
- 関連: PRD §10 (配布) / PRD 成功指標 3

> **この判断は誤っていた。** 入力方式の登録は機械全体に書かれるため、
> 利用者ごとの登録では完結しない。詳しくは ADR-0007 を参照。

## 背景

TSF の TIP を使えるようにするには、COM のクラスを登録し、入力方式として
登録する必要がある。TSF のサンプルや既存の実装はいずれも
`HKEY_LOCAL_MACHINE\Software\Classes\CLSID` に書いており、これには管理者権限が要る。

一方、開発中は DLL を何度も作り直して入れ替える。そのたびに権限昇格の確認が
出るのは、単に遅いだけでなく、確認を惰性で押す癖がつく点でも良くない。

COM は `HKEY_CURRENT_USER\Software\Classes` も見るため、利用者ごとの登録は
可能である。入力方式の登録 (`ITfInputProcessorProfileMgr::RegisterProfile`) は
もともと利用者ごとに行われる。

## 選択肢

### A. 常に HKLM に登録する

- 利点: 配布時と開発時で登録の仕組みが同じ。すべての利用者と、昇格した
  プロセスから見える。
- 欠点: 登録・解除のたびに管理者権限が要る。開発の反復が遅くなる。

### B. 開発中は HKCU、配布時は HKLM

`DllRegisterServer` は HKCU に書く。インストーラは別途 HKLM に書く。

- 利点: `regsvr32` が権限昇格なしで通る。開発の反復が速い。外部の
  コントリビューターが手元で試すのも容易になる (PRD 成功指標 3)。
- 欠点: 登録の経路が二つになる。HKCU に登録した状態では、他の利用者や
  SYSTEM として動くプロセスから CrystalSKK が見えない。

## 決定

**B を採る。** `DllRegisterServer` は `HKEY_CURRENT_USER\Software\Classes` に
書き、`regsvr32` が管理者権限なしで動くようにする。

配布用インストーラが HKLM に書く必要が出た時点で、そちらは別の仕組みとして
用意する。`DllRegisterServer` の挙動は変えない。開発中にインストーラ経由でしか
試せなくなるのは、反復の速さという利点を捨てることになるため。

## 帰結

- `regsvr32 crystalskk_tip.dll` が昇格なしで通る。解除も同じ。
- HKCU に登録した状態では、**他の利用者や SYSTEM として動くプロセス
  (ログオン画面など) から CrystalSKK が見えない**。開発中は問題にならないが、
  「なぜかあの画面では出ない」と悩む前にここを思い出すこと。
- 登録の経路が二つになる。両方が同じ内容を書くことを、インストーラを作る
  段階で確かめる必要がある。
- GUID を変えると古い登録が孤児として残る。[`guids.rs`](../../crates/crystalskk-tip/src/guids.rs)
  の値は変えない。
