# Phase 3 の移行と検証

日本語（正本） | [English reference translation](MIGRATION_PHASE_3.md)

> 日本語版を正本とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。
>
> checkpoint の形式は [Phase 4](MIGRATION_PHASE_4_ja.md)（v4）で置き換わりました。

Phase 3 では、永続的な Effect journal と書き込みの再試行規則を加えた。正となる checkpoint は
[`execution-v3.schema.json`](schema/execution-v3.schema.json) である。形式 v1 / v2 は歴史的なものとして残り、
`ExecutionState::restore` は拒否する。送信済みの書き込みと未着手の書き込みを区別できない場合があるため、
Effect の記録をでっち上げて移行することはできない。古い実行は調査用に残し、新しい実行を明示的に始める。
`RunStore::open` は v3 だけを受け付け、checkpoint の実行 ID を、そのディレクトリと（あれば）`state.json` に照らして確かめる。

すべての MCP の route は `effect_class`（`pure`、`read`、`local_write`、`external_write`、`destructive`）を宣言すべきである。
MCP で省略した場合は `external_write`、モデルで省略した場合は `pure` として扱う。external / destructive の route には、
provider が対応する `idempotency_argument` か、明示的な `manual_reconciliation = true` のどちらかが必要である。
後者は最初の dispatch を許し、結果が曖昧になった後の再試行の前に、信頼された照合を求める。引数名は設定が所有し、
主制御が渡したキーは拒否する。例の routing ファイルは、既存の action にクラスを宣言している。MATLAB
（`calculation_graphing`）と KDB（`table_analysis`）は `local_write` である。研究室が所有するホストで動き、コードの
エラーが返っても、主制御が修正して再実行するループを止めてはならないためである。Codex を使う `open_data_acquisition`
は、手動の照合付きの `external_write` のままとする。宣言したクラスは信頼された設定上の主張であり、ツールの sandbox や
provider の振る舞いの証明ではない。capability の強制は Phase 5 で扱う。

`EffectRequest` は、オーケストレータが生成する provider のキーを除いた正規化 JSON の引数を SHA-256 でハッシュする。
journal は、action、resource、クラス、payload のハッシュ、invocation ID、安定した冪等キーを保持し、監査の JSONL に生の引数は
加えない。invocation の結果と Effect の確認結果は checkpoint に保存され、ワークフローのデータとしての機密区分を引き継ぐ。
キーは `awhdl:<run UUID>:<effect UUID>` で、`not_found` の照合が証明された後、その同じ Effect に再利用する。
同じ operation / 世代で payload や resource が変われば、新しいキーを持つ新しい Effect になる。external と destructive の
クラスでは、同じ operation の別の Effect が `STARTED` か `UNCERTAIN` の間、この新しいインスタンスを拒否する。
同じインスタンスのクラスが変わることも拒否する。

`NOT_STARTED` は、呼び出しを準備する前に永続化する。対応する invocation と `STARTED` は、機器の future を poll する前に
まとめて確定する。通常の成功では、invocation の完了と `CONFIRMED` をまとめて確定する。確認済みの重複は保存した結果を返す。
external / destructive の書き込みでは、dispatch 後の通信・意味上のエラー、provider のタイムアウト、直列化の失敗は
すべて `UNCERTAIN` になる。`local_write` では、同じく報告されたエラーは `FAILED` になり、再試行できる。
`RunStore::open` は、永続化された `STARTED` を `UNCERTAIN` に変え、その遷移を永続化してから呼び出しを許す。
`UNCERTAIN` の Effect は、provider のキーがあっても、盲目的に再試行することはない。

信頼された `reconcile_effect` は、provider の証拠に基づいて `confirmed(result)`、`not_found`、`still_uncertain` を受け付ける。
`not_found` は、destructive でない Effect を元のキーで再試行することを許す。destructive の再試行は禁止のままである。
`RunStore::open` と照合は runtime の API であり、CLI での再開と provider の自動照会は実装していない。通信の失敗の後に
`confirmed` の照合があった場合、その試行の invocation は失敗のままとし、provider が確認した Effect の結果を記録する。
この場合、利用側は失敗した通信を成功した invocation と扱うのではなく、Effect の結果を使うべきである。

現在の逐次エンジンの MCP 呼び出しは、設定された書き込みクラスでこの journal を使う。モデルと読み取り専用の MCP の呼び出しは、
通常の invocation の意味論を使い続ける。メモリ上の `successful_mcp_calls` のフィルタは最適化にすぎず、確認済みの Effect を
再起動をまたいで重複排除するのは journal だけである。既存の MCP が宣言する `read` クラスは設定上の主張であり、
許可リストのツールが変わったときは見直すこと。`manual_reconciliation` は人間の承認ではない。action に結び付いた承認と
capability の検査は、後の Phase で扱う。

必要な検証（`awhdl/` で実行）：`cargo test --workspace --offline`、`cargo fmt --check`、
`cargo clippy --workspace --all-targets --offline -- -D warnings`。オフラインの `dataflow_checkpoint` の例は、スキーマ検証用の
v3 の checkpoint を出力する。これらのテストに実際の provider の呼び出しは不要である。
