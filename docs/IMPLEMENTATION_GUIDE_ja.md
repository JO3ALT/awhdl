# 実装ガイド

日本語（正本） | [English reference translation](IMPLEMENTATION_GUIDE.md)

> **非規範。** 本ガイドは実装の作り方を説明する。言語・runtime・セキュリティの意味論も範囲も定めない。規範文書:
> [LANGUAGE_SPEC](LANGUAGE_SPEC_ja.md)、[RUNTIME_SPEC](RUNTIME_SPEC_ja.md)、[SECURITY_SPEC](SECURITY_SPEC_ja.md)、
> [IR_SPEC](IR_SPEC_ja.md)、[PROFILE_v0.1](PROFILE_v0.1_ja.md)。v0.1 の範囲を定めるのは PROFILE_v0.1 だけである。
> 日本語版を正本とし、英語版は参考訳とする。

## 貢献者と coding agent のための範囲の規律

1. 最初に PROFILE_v0.1 を読む。Profile が `v0.1` の機能だけを実装する。
2. 仕様や古い指示書に記述があっても、`v0.2` の機能や Profile にない機能は実装しない。先に Profile の変更を
   設計判断の記録とともに提案する。
3. 機能の状態を変える変更では、同じ変更で表の行と Evidence を更新する。実装の主張が実在するテストを名指さない
   場合、`profile_matrix_is_backed_by_existing_tests` が失敗する。
4. 曖昧な点は黙って解釈せず、判断とその理由を [設計判断](REFINEMENT_DECISIONS_ja.md) に記録して解決する。
5. LLM や agent の出力を信頼された control plane の一部にしない。

## Rust workspace

| ディレクトリ | パッケージ（バイナリ） | 役割 |
|---|---|---|
| `awhdl-ast` | `awhdl-ast` | AST の型 |
| `awhdl-parser` | `awhdl-parser` | pest 文法（`awhdl.pest`）と parser |
| `awhdl-checker` | `awhdl-checker` | 静的検査（構造 `AWHDL-E2xx`、情報フロー `E3xx`、Profile `E4xx`） |
| `awhdl-graph` | `awhdl-graph` | 検査済み設計のビュー射影と Mermaid・DOT・JSON 出力 |
| `conductor-cli` | `conductor-cli`（`aic`） | AWHDL ソースの `aic check`・`aic graph` |
| `aiconductor-runtime` | `aiconductor` | Tokio の runtime: 実行状態、dataflow、Effect、承認、capability、完了、AWHDL 設計のコンパイルと実行、engine、MCP / LLM クライアント、CLI |

Rust edition 2024、非同期 runtime は Tokio、CLI 解析は clap、parser は pest。

## テストの方針

- すべての変更で必須（リポジトリの root で実行）:
  `cargo test --workspace --offline`、`cargo fmt --check`、
  `cargo clippy --workspace --all-targets --offline -- -D warnings`。
- 単体テストは live の provider を呼ばない。意味論は `ExecutionState` の API と、一時ディレクトリを使う
  `RunStore` で検査する。
- runtime のテストは同梱の匿名化した設定（`crates/aiconductor-runtime/tests/fixtures/project`）で動き、
  このリポジトリだけで完結する。周囲に配備用プロジェクトがある開発マシンでは、その本番設定も読み込んで検証し、
  route と capability の定義が fixture と一致することを確認する。
- checkpoint の形式は、offline の `dataflow_checkpoint` example を使い `docs/schema` に対して検証する。
- live の確認はテストではなく example で行う。`live_matlab_smoke` は MATLAB MCP を engine の adapter・capability・
  Effect の経路で動かし、`live_approval_smoke` は HumanPort の承認 GUI を動かす。配備用プロジェクトでは
  `aiconductor run-design <design.awhdl> --input name=value` がコンパイルした AWHDL 設計を実機で実行する。
- interpreter の意味論は scripted な `Dispatcher` で、承認の流れは scripted な `Approver` で検査する。HumanPort の
  承認者モードは、実際のスクリプトの MCP 側だけを GUI なしで起動して検査する（HumanPort がなければスキップ）。

## 過去の指示書

配備用プロジェクトにある `CODEX_IMPLEMENTATION_INSTRUCTIONS.md` と、このリポジトリの
`IMPLEMENTATION_GUIDE_v0.2*.md` は以前の実装指示書である。そのマイルストーンと範囲の記述は PROFILE_v0.1 により
置き換えられた。
