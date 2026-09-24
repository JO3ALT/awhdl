# Phase 2 の移行と境界

日本語（正本） | [English reference translation](MIGRATION_PHASE_2.md)

> 日本語版を正本とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。

Phase 2 の記録（履歴）です。Phase 3 で checkpoint の形式 v2 が置き換わり、Effect journal が加わりました。
[Phase 3 の移行](MIGRATION_PHASE_3_ja.md) を参照してください。

Phase 2 では、`aiconductor-runtime` の中で、最新の値、一度きりのイベント、invocation のライフサイクルを分離した。
AWHDL の parser / AST は Milestone 1 のままである。ドットを含む感度名は、型のない文字列としてすでに構文解析され、
先頭の名前の構造検査だけを受ける。構文解析できても、発火の意味論が定まるわけではない。新しい `event` 宣言は未対応のまま。

- 新しい checkpoint は `execution-v2.schema.json` を使う。バージョン 1 は歴史的なスキーマとして残し、
  `ExecutionState::restore` は拒否する。自動の移行はない。v1 にはイベントの identity、消費の履歴、結果の payload がないためである。
  古い実行は調査用に残し、新しい実行は明示的に始める。欠けた状態を再構成するために、消費済みのイベントを作り出したり、
  保留中の呼び出しを再送したりしてはならない。
- `Invocation<T>` が具体的なレコード構造を置き換える。`InvocationRecord` はその JSON 版の別名である。
  `ResultEnvelope<T>` はアダプタの戻り値の型のまま。`RunStore::invoke` は `T: Serialize` を要求し、成功した結果を
  返す前に checkpoint に保存する。既存の planner / model / MCP の呼び出し箇所はこの制約を満たす。
- 成功した JSON の null は `result: {"payload": null}` と表し、結果がない場合は `result: null` とする。
  保留・失敗・取り消しの invocation には結果がない。
- `execution.json` と、`state.json` に埋め込まれたスナップショットは payload を含む。これらはメタデータだけの監査ログではなく、
  入出力の機密区分を持つワークフローのデータとして扱わなければならない。監査の JSONL に payload の項目は追加しない。
  この Phase で、既存の機密区分と capability の強制を広げてはいない。
- 永続的な変更には `RunStore::assign_value`、`publish_event`、`consume_event` を使う。対応する `ExecutionState` の API は
  メモリ上の状態しか変えない。古いスナップショットから復元すると、古い作業を再実行するおそれがある。常に正となる
  `execution.json` から復元し、所有する実行の ID を確かめ、1 つの実行の所有者は 1 つに限ること。
- 消費は配送の前に永続化する（at-most-once）。その直後にクラッシュすると、処理が失われることがある。
  副作用の exactly-once、イベントハンドラの原子的な実行、外部からの入力の重複排除、照合（reconciliation）は約束しない。
  これらには Phase 3 の Effect のプロトコルが必要である。消費済みのイベントの履歴を切り詰めてはならない。
- 永続化でエラーが起きると、`RunStore` はそれ以降の遷移と dispatch を拒否する。永続化された checkpoint を調べて復元すること。
  自動の再試行や CLI での再開はない。取り消しは runtime の状態だけを変え、動いている provider の処理は止めない。
- 現在の逐次エンジンの完了経路は、invocation の結果とライフサイクルのイベントを永続化する。ただし同期的な戻り値の
  envelope を使ったままで、キューから AWHDL の process をスケジュールすることはない。値の伝播、timer / webhook / 承認の
  アダプタ、購読、並列スケジューリング、delta cycle の実行はここでは実装していない。汎用の信頼された入力経路から
  外部イベントを発行することはできる。

回帰テストは、値の変わらない代入、内容の等しい別々のイベント、消費済みイベントからの再開、invocation ごとの完了、
JSON の null の結果、取り消し、古い世代の分離、壊れた checkpoint の拒否、書き込み失敗の扱いを対象とする。
必要な検査は `awhdl/` で実行する。

```sh
cargo test --workspace --offline
cargo fmt --check
cargo clippy --workspace --all-targets --offline -- -D warnings
```

`cargo run -p aiconductor --example dataflow_checkpoint --offline` は、機器に接続せずに代表的な v2 の checkpoint を出力する。
その出力を、Draft 2020-12 の検証器で `schema/execution-v2.schema.json` に照らして検証する。検証結果は進捗を参照のこと。
