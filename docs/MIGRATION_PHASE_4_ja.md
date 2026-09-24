# Phase 4 の移行と検証

日本語（正本） | [English reference translation](MIGRATION_PHASE_4.md)

> 日本語版を正本とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。
>
> checkpoint の形式は [Phase 5](MIGRATION_PHASE_5_ja.md)（v5）で置き換わりました。

Phase 4 では、人間の承認を 1 つの具体的な Effect インスタンスに結び付けた。正となる checkpoint は
[`execution-v4.schema.json`](schema/execution-v4.schema.json) である。形式 v1〜v3 は歴史的なものとして残り、
`ExecutionState::restore` は拒否する。v3 の checkpoint は、承認の記録をでっち上げて移行することはできない。
送信済みの外部書き込みのどれを人間が認めたかを示せないためである。古い実行は調査用に残し、新しい実行を明示的に始める。

## 振る舞いの変更

- Effect は `requires_approval` を記録する。`EffectRequest::new` は、`external_write` と `destructive` で既定を true にする。
  `with_approval(false)` で除外できるが、`destructive` は除外できない。
- `start_effect` は、再計算した正規化 action とハッシュが一致する、期限内の承認を、`STARTED` と同じ遷移で消費する。
  承認がなければ遷移は失敗し、何も永続化しない（invocation も STARTED も provider の呼び出しもない）。
- `RunStore` に `reserve_effect`、`request_approval`、`decide_approval` を加えた。呼び出し側は Effect を予約し、
  正確な引数で承認を求め、信頼された判断を得てから `invoke_effect` を呼ぶ。
- route の設定に `human_approval` を加えた。`open_data_acquisition`（Codex、`external_write`）は明示的に `not_required` とし、
  承認のアダプタができるまで Phase 4 以前の振る舞いを保つ。MATLAB と KDB は `local_write` で影響を受けない。
  設定のない新しい `external_write` の MCP route は、fail closed になる。

## 制限

- エンジンに承認のアダプタは接続していない。HumanPort は runtime の MCP 一覧になく、承認者として信頼するには、
  その `human.answer` ツールをエージェントから見える範囲と分離する必要がある。
- `transaction` と `time_limited_session` のスコープは実装していない。
- Phase 5 がパラメータ付きの capability を定めるまで、`capability` は `<class>:<resource>` である。
  承認は後の capability の検査を回避しない。
- 正規化の符号化は、キーを整列した serde_json であり、RFC 8785 ではない。
- 承認の期限はオーケストレータの時計を使う。

## 検証

`awhdl/` で：`cargo test --workspace --offline`、`cargo fmt --check`、
`cargo clippy --workspace --all-targets --offline -- -D warnings`。オフラインの `dataflow_checkpoint` の例は、
消費済みと保留中の承認を含む v4 の checkpoint をスキーマ検証用に出力する。実際の provider も人間も不要である。

> **更新（2026-09-23）：** HumanPort の承認アダプタを接続し、`open_data_acquisition` の除外を撤廃した。
> 公開データの取得は、毎回 HumanPort の画面で人間の判断を待つ。
