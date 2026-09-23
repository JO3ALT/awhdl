# AWHDLの設計判断

[English](DESIGN_DECISIONS.md) | 日本語

> この日本語版を規範文書（正本）とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。

## 正式名称

AWHDLの正式名称は **Agentic Workflow Harness Description Language**（エージェント型ワークフロー・ハーネス記述言語）です。HはHardwareではなくHarnessを表します。Harnessは、複数AI、MCP、決定的ツール、人間を接続し、データフロー、event、反復、予算、権限、完了条件、安全境界を一つの検査・実行可能な構造として束ねるという設計意図を表します。

VHDLは並行性、signal、process、event、構造的検査の着想元ですが、AWHDLは電子回路を記述せず、hardware synthesisを目的としません。

## Milestone 1の範囲

Milestone 1は、標準的な`hello.awhdl`の垂直スライスに必要な構文だけを実装します。対象は、エンティティとポート、アーキテクチャ、単純なデバイスおよび信号宣言、プロセス、デバイス呼び出し、信号代入、`null`です。

ASTはすでに機密区分の表記とソース上のバイト範囲を保持しています。そのため、Milestone 2の検査器はパーサー境界を置き換えずに機密区分ラティスを追加できます。

## 解析と診断

パーサーはPestを使用し、AWHDLのキーワードを小文字で受理します。AWHDLをVHDL完全互換とは見なしません。実装範囲外の構文は`AWHDL-E101`として拒否し、黙って無視しません。

検査器は名前と構造のみを検査します。`E2xx`診断は、ポートの型検査やセキュリティ情報フロー解析が実装済みであることを意味しません。

## 統合ランタイムとの境界

`aiconductor-runtime`統合プロトタイプは独立したままとし、規範的なAWHDLコンパイラパイプラインには含めません。Milestone 1はAWHDLをランタイム設定へコンパイルせず、設計を実行しません。統合はバージョン付きIRを導入した後に行います。

## 保留中の論点

- キーワードの大文字・小文字区別とUnicode識別子規則
- 完全な式文法とオーバーロード解決
- エンティティ／アーキテクチャ終了名の一致
- 複数ドライバーと信号解決
- generic map、デバイスポート宣言、`if`、`case`、タイマー、予算、並列ブロック
- 機密区分ラティス、テイント、機密解除、クラウド送信ポリシー

## Phase 1 refinement (2026-09-23)

Integration runtime identity, checkpoint and barrier APIs are implemented separately
from the AWHDL compiler. See [decisions](REFINEMENT_DECISIONS_ja.md) and
[migration / limits](MIGRATION_PHASE_1.md). This does not add AWHDL
compile/run, signal scheduling or a CLI resume command. Workspace verification
passed on 2026-09-23: 33 tests, format check and Clippy with warnings denied.

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

## v0.1 backlog の実装（2026-09-23）

v0.1 構文、静的な情報フロー検査、route 束縛を伴う runtime へのコンパイル、delta cycle
実行、HumanPort 承認 adapter、MATLAB ファイル実行を実装した。状態は
[PROFILE_v0.1](PROFILE_v0.1.md)、判断は [設計判断](REFINEMENT_DECISIONS_ja.md) を参照。
