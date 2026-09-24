# 実行 IR 仕様

日本語（正本） | [English reference translation](IR_SPEC.md)

> **正本の範囲。** 規範文書: [LANGUAGE_SPEC](LANGUAGE_SPEC_ja.md)、
> [RUNTIME_SPEC](RUNTIME_SPEC_ja.md)、[SECURITY_SPEC](SECURITY_SPEC_ja.md)、
> [IR_SPEC](IR_SPEC_ja.md)、[PROFILE_v0.1](PROFILE_v0.1_ja.md)。非規範:
> [IMPLEMENTATION_GUIDE](IMPLEMENTATION_GUIDE_ja.md)、例、チュートリアル、移行メモ、状態報告。
> 矛盾時は PROFILE_v0.1、次に各規範文書が優先する。v0.1 の実装範囲は PROFILE_v0.1 だけが定める。
> 日本語版を正本とし、英語版は参考訳とする。差異がある場合は日本語版が優先する。

範囲: 版管理、invocation の metadata、action 記述、Value / Event の符号化、Effect と承認の状態、checkpoint の形式。

現在の形式: v5（実行、Effect、承認、capability の bind）。機械可読な schema:
[`schema/execution-v5.schema.json`](schema/execution-v5.schema.json)。
[`schema/`](schema) の v1〜v4 は履歴であり、現在の restore はそれらを拒否する。これは runtime の状態であり、
AWHDL のプログラム IR ではない（下記「コンパイル済み設計」を参照）。

`ExecutionState` は `version: 5`、`workflow_run_id`、`correlation_id`、`generation`、`records`（invocation の
UUID → `Invocation<JSON>`）、`dataflow`、`effects`（Effect の UUID → Effect）、`approvals`（承認の UUID → 承認）を
直列化する。record は `identity`、`operation`、`status`、`result` を持つ。status は `pending`、`completed`、`failed`、
`cancelled`。結果を持つのは completed の record だけであり、成功した JSON null が直列化後も残るよう
`{"payload": T}` で包む。それ以外の結果は null である。`ResultEnvelope<T>` は device の応答を呼び出し元へ返すための
`{identity, payload}` であり、保存される Value や Event ではない。

identity のフィールド: `workflow_run_id`、`invocation_id`、`parent_invocation_id`、`causation_id`、`correlation_id`、
`generation`（u64）、`attempt`（正の u32）、`created_at`（UTC の時刻）。null 可の parent は invocation を、null 可の
cause は同じ scope の既存 Event を参照する。ID は runtime が所有する。Phase 1 の identity の所有、retry、
generation の規則が引き続き適用される。

`dataflow` は以下を含む。

- `values`: 名前 → `Value<JSON>`。`scope`、正の `revision`、`payload` を持つ。scope は
  `{workflow_run_id, correlation_id, generation}`。名前ごとに最新の代入を保持し、参照は現在の generation に
  限る。revision は各 generation で 1 から始まり、JSON の値が変わったときだけ増える。
- `events`: 追加順の `Event<JSON>` の履歴。各イベントは `event_id`、正で連続した `sequence`、`scope`、null 可の
  `causation_id`、`kind`、`payload` を持つ。`kind` はタグ付きオブジェクトである:
  `{"kind":"external","topic":"timer"}`、`{"kind":"value_changed","name":"candidate","revision":1}`、
  `{"kind":"invocation_terminated","invocation_id":"<UUID>","status":"completed"}`。終端 status には failed /
  cancelled もある。ライフサイクルイベントの payload は null で、結果は invocation の record にある。変更イベントの
  payload は代入された値のスナップショットである。外部の payload は任意の JSON でよい。
- `consumed`: 永続的な消費の印として保持する一意のイベント ID。イベントの履歴は検証と起因の参照に使い、
  未消費の値として扱うことはない。

restore は、版、run / correlation の scope、invocation の identity と親、attempt の一意性、結果と status の整合、
イベントの連続した順序、ID の一意性、同じ scope の先行イベントへの起因、既知の消費済み ID、値の revision と内容の
整合、終端した invocation ごとに一致するイベントがちょうど1つあることを検証する。古い scope は履歴に残ってよいが、
現在のイベントとして配送することはできない。schema の検証だけでは、これらの record をまたぐ不変条件は確認できない。

checkpoint は payload を含む。metadata のみの監査 JSONL は別である。

各 Effect は `effect_id`、`scope`、`operation`、`action`、`resource`、`payload_hash`（正規化した JSON 引数の
SHA-256 を小文字 16 進 64 文字で表したもの）、`class`、`capability`、`idempotency_key`、`state`、
`requires_approval`、null 可の `invocation_id`、null 可の `result`（確定時は JSON null を含め `{"payload": T}`）を
持つ。Effect の class は `pure`、`read`、`local_write`、`external_write`、`destructive` であり、journal が受け付けるのは
後の3つである。state は `not_started`、`started`、`confirmed`、`failed`、`uncertain`。pure / read の呼び出しは
Effect を持たない invocation の record である。安定した key は `awhdl:<workflow_run_id>:<effect_id>` である。

`NOT_STARTED` は invocation を持たない。`STARTED` は pending の invocation を指す。`CONFIRMED` は結果を持つ。
`UNCERTAIN` は遠隔書き込みの transport 失敗、または復元された実行中の書き込みを含み、dispatch を止める。`FAILED` は、
provider がその操作を行わなかったという信頼された照合の結論、または報告された `local_write` のエラーを表す。
Effect は (generation, operation, action, resource, payload_hash) ごとに最大1つである。restore は、永続化された
`STARTED` をメモリ上で `UNCERTAIN` に変える前に、Effect の identity と scope、key、記述の形、結果と状態の整合、
参照する invocation を検査する。`RunStore::open` はその復旧の遷移を永続化する。retry と機密性の規則は
[Phase 3 の移行メモ](MIGRATION_PHASE_3_ja.md) を参照。

## 承認（v4）

各 Effect は `requires_approval`（真偽値）を持つ。最上位の `approvals` map は `approval_id`、`effect_id`、`scope`
（`single_action` のみ）、`action_hash`、`content_hash`（Effect の `payload_hash` と等しい）、`capability`、
`resource`、`class`、`generation`、`issued_at`、`expires_at`（発行から最大 24 時間）、`state`（`pending`、`granted`、
`denied`、`consumed`）、null 可の `decided_at`（pending でないときに限り設定）、null 可の `consumed_by`
（消費した invocation。consumed のときに限り設定）を持つ。

`action_hash` は次の記述を serde_json で符号化（キーは整列）したものの SHA-256 を小文字 16 進で表したものである。

```json
{"schema": "awhdl.action.v1", "workflow_run_id": "...", "correlation_id": "...",
 "generation": 0, "effect_id": "...", "operation": "...", "action": "...",
 "capability": "<capability id>#<fingerprint>", "resource": "...", "class": "...",
 "content_hash": "<payload_hash>"}
```

この符号化は本 runtime において決定的であるが、RFC 8785 JCS であるとは主張しない。`capability` は Effect を認可した
capability の handle である（Phase 5）。restore はすべての承認の hash を対応する Effect から計算し直し、不一致を
拒否する。また、承認が必要な Effect のすべての dispatch が、ちょうど1つの承認の `consumed_by` であることを要求する。
[Phase 4 の移行メモ](MIGRATION_PHASE_4_ja.md) を参照。

## Capability の bind（v5）

各 Effect は `capability` を持つ。これは Effect を認可した capability の handle `<id>#<fingerprint>` であり、
fingerprint は grant の権限範囲（`id`、`action`、`resource`、`constraints`、`effect_class`。保持者と取り消し状態は
含まない）の SHA-256 の先頭 16 文字である。policy の判断を経ずに runtime API で作った Effect は `unscoped` を持つ。
承認の `capability` は Effect の handle と等しいため、grant の範囲が変われば action hash が変わり、承認は失効する。
同じ instance を別の handle で予約し直すことはできない。grant 自体は checkpoint ではなく設定
（`config/capabilities.toml`）にある。[Phase 5 の移行メモ](MIGRATION_PHASE_5_ja.md) を参照。

## コンパイル済み設計

`design::CompiledDesign`（区分と初期値を持つ信号、束縛した device、process、assertion、cloud の下限、timer、
barrier、上限）は直列化できるが、メモリ上のプログラムである。run ごとにソースから作り直し、v0.1 では永続化も
版管理もしない IR である。実行 checkpoint の形式は変わらない（v5）。
