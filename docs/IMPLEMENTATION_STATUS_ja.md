# AWHDL実装状況

> **範囲と状態の正本（Phase 7、2026-09-23）:** v0.1 の実装範囲と各機能の実装状態は
> [PROFILE_v0.1](PROFILE_v0.1.md) の表だけが定める。本書はその要約であり、
> 表と食い違う場合は表が優先する。本書は範囲を独自に定義しない。

[English](IMPLEMENTATION_STATUS.md) | 日本語

> この日本語版を規範文書（正本）とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。

## リリース状態（2026-09-23）

- 仕様: ドラフト v0.2。実装範囲: PROFILE_v0.1
- コンパイラ: parser、静的検査（構造 `E2xx`・情報フロー `E3xx`・Profile `E4xx`）、
  route への束縛を伴う Rust runtime へのコンパイル
- CLI: `aic check <file>`、`aiconductor run-design <file> --input name=value`
- ランタイム: process・timer・timeout・parallel・barrier・budget・assertion を
  delta cycle で実行。device 呼び出しは engine の capability・Effect・承認・監査の経路を通る

## 実装済みの構文

- エンティティ、ポート、アーキテクチャ
- `agent` / `mcp` / `deterministic` のデバイスと `generic (...)`（`route`・`location`・`clearance`）
- 機密区分付きの信号と定数初期値
- `timer`・`budget`・`barrier` 宣言
- `assert always (...)`・`assert never (...)`・`assert never (<区分> -> <配置>)`
- 感度リスト、process の `timeout` と `on timeout`
- `timeout` 付き可能なデバイス呼び出し、代入、`if`/`elsif`/`else`、`parallel`、逐次 `assert`、`null`
- `or`・`and`・`not`・比較・整数の `+`/`-`・リテラル・選択名を含む式

## 実装済みの検査

- 行・列付きの構文エラー、予約語
- 名前の重複・未定義、入力ポートへの書き込み、budget と timer の値、barrier の構成、parallel の書き込み競合
- 機密区分と配置を v0.1 Profile の範囲に限定
- restricted から cloud への流れ（`E301`）、clearance（`E302`）、暗黙の機密解除（`E303`）。区分のない型は restricted とみなす
- コンパイル時: デバイス種別、route への束縛、配置の一致

## 未実装

- Event 宣言と完了条件の構文（意味論は実装済み、構文は未定）
- `case`、`await`、FSM 型、`retry`、`approve`、`policy`、`export`、`configuration`、時相アサーション、testbench、fault injection
- 機密解除 device、confidential / secret、sandbox / private_cloud / external、条件分岐を通じた暗黙のフロー
- 永続化・版管理されたプログラム IR、`parallel` 内の同時 dispatch
- `aic compile`、`sim`、`graph`、`audit`

未対応の構文は無視せず、必ず診断を生成しなければなりません。

## 適合方針

仕様書は意図する言語を定義します。現在のプロトタイプが対応するのは、「実装済みの構文」と「実装済みの検査」に記載した機能だけです。以後のマイルストーンでは、実装およびテストと同じ変更内でこの文書も更新します。

## Phase 1 仕様整理（2026-09-23）

AWHDL compiler とは独立した統合ランタイムに、実行 identity、checkpoint 復元、
generation-aware barrier の API を実装した。
[設計判断](REFINEMENT_DECISIONS_ja.md) と
[移行・実装境界](MIGRATION_PHASE_1.md) を参照。
AWHDL compile/run、signal/event scheduler、CLI resume は未実装。
workspace 全33テスト、fmt、警告をエラーとする Clippy は成功した。

## Phase 2 仕様整理（2026-09-23）

統合ランタイムで `Value<T>` / `Event<T>` / `Invocation<T>` を分離した。
checkpoint v2 に最新値・イベント履歴と消費記録・呼び出し結果を保存する。
全 device 完了で lifecycle Event を生成するが、serial engine は既存の return を使う。
AWHDL scheduler・delta-cycle 実行・新構文・CLI resume は未実装。
[Runtime](RUNTIME_SPEC.md)、[設計判断](REFINEMENT_DECISIONS_ja.md)、
[移行](MIGRATION_PHASE_2.md)、
検証 を参照。

## Phase 3 仕様整理（2026-09-23）

書き込み種別の MCP route を永続 Effect journal に接続した。external / destructive
の書き込みで timeout 等により結果が不明なら照合まで再試行を拒否する。
`local_write`（MATLAB・KDB）の報告済みエラーは FAILED とし再試行できる。
確認済みの重複呼び出しは保存結果を返す。
checkpoint v3 は Effect identity と状態を保存する。parser / AST / structural
checker は変更していない。[Runtime](RUNTIME_SPEC.md)、
[設計判断](REFINEMENT_DECISIONS_ja.md)、[移行](MIGRATION_PHASE_3.md)、
検証 を参照。

## Phase 4 仕様整理（2026-09-23）

書き込み Effect の承認を、1つの action instance の正規化記述の SHA-256 に bind した。
承認は1回限り・期限付きで、`STARTED` と同じ checkpoint で消費する。external /
destructive は既定で承認必須、`open_data_acquisition` は明示的に除外。承認 adapter は
未接続のため承認必須 action は fail closed。checkpoint v4。parser / AST / structural
checker は変更していない。[Runtime](RUNTIME_SPEC.md)、
[設計判断](REFINEMENT_DECISIONS_ja.md)、[移行](MIGRATION_PHASE_4.md)、
検証 を参照。

## Phase 5 仕様整理（2026-09-23）

tool・ファイル・sandbox・network・model の権限を既定拒否の capability model
（`config/capabilities.toml`）に統合し、`authorize` で一元判定する。認可した grant の
class が tool 単位で journal・承認を決め、route の class は上限とする。planner が渡す
ファイルパスは正規化し許可されたパスに限る。Effect と承認は capability handle に bind。
checkpoint v5。parser / AST / structural checker は変更していない。
[Runtime](RUNTIME_SPEC.md)、[設計判断](REFINEMENT_DECISIONS_ja.md)、
[移行](MIGRATION_PHASE_5.md)、検証 を参照。

## Phase 6 仕様整理（2026-09-23）

planner の完了宣言は評価の依頼にすぎず、runtime が state machine・Effect journal・
typed adapter 出力から判定する。hard 条件は決定的、model 由来は soft、外部書き込みを含む
workflow は safety-critical。設定のみで checkpoint は v5 のまま。parser / AST /
structural checker は変更していない。[Runtime](RUNTIME_SPEC.md)、
[設計判断](REFINEMENT_DECISIONS_ja.md)、[移行](MIGRATION_PHASE_6.md)、
検証 を参照。

## Phase 7 v0.1 Profile（2026-09-23）

文書を役割ごとに再編した。v0.1 の範囲と各機能の状態は
[PROFILE_v0.1](PROFILE_v0.1.md) の表だけが定め、テストがコードとの対応を検査する。
本書はその要約。規範文書は LANGUAGE_SPEC・RUNTIME_SPEC・SECURITY_SPEC・IR_SPEC・
PROFILE_v0.1。[設計判断](REFINEMENT_DECISIONS_ja.md)、[移行](MIGRATION_PHASE_7.md) を参照。
