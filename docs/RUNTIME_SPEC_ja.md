# Runtime 仕様

日本語（正本） | [English reference translation](RUNTIME_SPEC.md)

> **正本の範囲。** 規範文書: [LANGUAGE_SPEC](LANGUAGE_SPEC_ja.md)、
> [RUNTIME_SPEC](RUNTIME_SPEC_ja.md)、[SECURITY_SPEC](SECURITY_SPEC_ja.md)、
> [IR_SPEC](IR_SPEC_ja.md)、[PROFILE_v0.1](PROFILE_v0.1_ja.md)。非規範:
> [IMPLEMENTATION_GUIDE](IMPLEMENTATION_GUIDE_ja.md)、例、チュートリアル、移行メモ、状態報告。
> 矛盾時は PROFILE_v0.1、次に各規範文書が優先する。v0.1 の実装範囲は PROFILE_v0.1 だけが定める。
> 日本語版を正本とし、英語版は参考訳とする。差異がある場合は日本語版が優先する。

範囲: invocation の identity、generation、correlation、Value / Event / Invocation、イベント配送、barrier、
cancellation、retry、checkpoint とクラッシュ復旧、Effect の意味論、完了評価、AWHDL 設計の実行。
本書の実行意味論は、言語仕様とシステム仕様における runtime の記述より優先する。
機械可読な形式は [IR_SPEC](IR_SPEC_ja.md) が定める。

## Identity と所有

信頼された runtime は、論理的なワークフロー入力ごとに1回だけ `workflow_run_id` と `correlation_id` の UUID を
生成する。`RunStore.run_id` は `workflow_run_id` と同じ識別子であり、別の名前空間ではない。複数の入力には
別々の実行コンテキストが必要である。新しいコンテキストは generation 0 から始まる。

実際の LLM または MCP の dispatch ごとに、新しい `invocation_id`、null 可の `parent_invocation_id`、継承した
correlation / generation、UTC の `created_at`、1 始まりの `attempt` を持つ。retry は operation slot を保ち attempt を
増やす。論理的な改訂は明示的に generation を進め、slot の attempt をリセットする。fallback provider は別の
attempt である。planner の新しいターンは新しい operation slot（`planner:<iteration>`）を使い、出力不正による
再試行はその slot を再利用する。action の device は `action:<configured-action-name>` を使う。兄弟関係にある
parallel 分岐は別々の operation slot を使い、親のコンテキストを継承する。親は同じコンテキストと generation に
存在しなければならない。ルートの呼び出しの親は null である。

これらのフィールドは AWHDL のリテラルでも planner が制御する引数でもない。adapter は応答を、信頼できない出力内の
フィールドではなく、要求時にローカルに保持した identity に結び付ける。`finish` / `finish_result` は identity 全体を
照合し、pending の記録を要求する。外部の、古い、偽造された、未知の、重複した完了は、payload が消費される前に
拒否する。失敗も identity を保持する。

## Barrier

実行状態の barrier API は、必要な operation slot を名指しする。現在の run / correlation / generation で、すべての
slot に完了した invocation があるときに限り ready となる。欠けた slot や失敗した slot では ready にならない。
他の generation、未知の identity、想定外または重複した slot、置き換えられた attempt はエラーである。保存された
payload があるだけで barrier が ready になることはない。

AWHDL の `barrier` 構文は、下記「AWHDL 設計の実行」の interpreter が信号単位で実行する。すべてのメンバー信号が
前回の発火以降に書き込まれたとき `barrier.ready` を発生させる。この interpreter の barrier は上記の永続 API
とは独立している。

## Checkpoint と復旧

`execution.json` が実行 checkpoint の正本である（形式 v5）。device の future を poll する前に、pending の identity を
一時ファイルへ書き、sync し、rename し、ディレクトリを sync する。完了、その結果、その終端イベントは検証のうえ
1つの checkpoint に永続化してから結果を返す。`state.json` も identity のスナップショットを含むが、呼び出し中は
`execution.json` より遅れる場合がある。復旧では後者を優先し、run ID を所有する run ディレクトリ / state と照合する。

`ExecutionState::restore` は版と identity の整合を検査する。pending の記録は pending のまま残り、identity の復旧は
device への要求を再発行しない。後から届いた信頼できる応答は、その identity を完了できる。identity の復元は
呼び出しの再実行でも exactly-once 実行の保証でもない。外部で成功した後、ローカルに永続化する前のクラッシュは、
下記の Effect 照合を必要とする。呼び出しを自動的に再実行してはならない。統合 CLI には現在 resume コマンドがない。
checkpoint の復元は runtime API としてテストしており、CLI からの再開としてはテストしていない。

## 統合の境界

planner 駆動の engine（`aiconductor run`）は serial であり、1つの論理入力を generation 0 で実行する。AWHDL 設計は
別の経路（`aiconductor run-design`、下記）で delta cycle により実行する。generation を進める API と barrier API は
将来のスケジューラに備えたものである。device dispatch の3つの経路（planner、fallback・清書を含む model、MCP）は
すべて `RunStore::invoke` を通る。model のヘルスチェックと起動管理は control plane の操作であり、ワークフローの
device invocation ではない。重複した MCP 呼び出しの抑止は新しい呼び出しを dispatch しないため、device invocation を
割り当てない。

envelope は、既存 engine が payload を消費する前に検証する。runtime の invocation の成功は transport の完了を表し、
意味上の成功は既存の adapter と action の検査が判定する。

## Value、Event、Invocation（Phase 2）

`Value<T>` は最新の状態であり、scope、revision、payload を持つ。代入はソースの文字列やモデルの類似度ではなく
`serde_json::Value` の等価性で比較する。最初の代入（null を含む）は `ValueChanged` を発生させる。同じ generation での
等しい値の代入は何も発生させず revision を保つ。異なる値は revision を増やし、変更イベントに payload の
スナップショットを追加する。新しい generation では明示的に代入するまで値は見えない。したがって古い generation と
等しい payload も新しい最初の代入となる。読み取りは値を消費しない。

`Event<T>` は1回限りの発生であり、runtime が生成する UUID、単調増加の sequence、scope、任意のイベント起因
（causation）、payload を持つ。等しい payload を2回 publish すれば2つの発生となる。キューの履歴と消費の記録は
別である。消費済みの履歴は process を再び起動できない。`next_event` は現在の scope で最も古い未消費のイベントを
選ぶ。信頼された経路は特定の ID を消費でき、同じ消費を繰り返してもイベントは返らない。未知または古い ID は
エラーである。再度 publish すると新しい identity が割り当てられる。外部からの流入は、新しい発生を作るのではなく、
再配送をまたいでイベントの identity を保たなければならない。

`Invocation<T>` は1つの完全な invocation identity に結び付いた pending / completed / failed / cancelled の
ライフサイクルである。完了が名前付きの Value を暗黙に代入することはない。遷移は pending から終端への1回だけで、
一致する現在の identity に限られる。終端遷移は、呼び出し元に公開する前に、成功時の結果とともに
`InvocationTerminated` イベントをちょうど1つ追加する。終端イベントの payload は null であり、その invocation ID が
保存された結果を指す。cancellation は runtime の遷移であり、provider 側で取り消された証明ではない。cancel 後に
届いた結果は拒否する。古い真偽値の `finish` API は成功時に JSON null を記録する。本番の adapter は直列化した結果を
`finish_result` で記録する。

`begin_caused` は、同じ scope の既存イベントに `causation_id` で invocation を結び付けられる。その終端イベントは
起因を継承する。親 invocation と起因イベントは別の関係であり、どちらもモデルの応答から与えることはできない。
一般の外部 publish で特権的なライフサイクル種別を作ることはできない。

## Process の感度と delta cycle

以下は別々の意味上のトリガーであり、同義ではない。

| 意図する感度 | runtime のトリガー |
|---|---|
| `process(signal.changed)` | 名前付き Value の `ValueChanged` |
| `process(event)` | 対応する topic 上の未消費の外部 Event |
| `process(invocation.completed)` | 一致する invocation かつ completed 状態の `InvocationTerminated` |

failed / cancelled のライフサイクルは completed の感度を満たさない。Value が存在するだけではイベントにならない。

AWHDL 設計の interpreter（下記）は、この区別に従って `name` / `name.changed`（値の変更）、`device.done` /
`device.failed` / `device.timeout`（呼び出しの終端）、timer、`barrier.ready` のトリガーを実行する。これらは
interpreter のメモリ上のイベントであり、上記の永続 Event store にはまだ接続していない。`event` 宣言と
`.completed` の表記は、構文が未定のため受け付けない（LANGUAGE_SPEC）。

delta cycle は、1つの論理的なスケジューリング単位の中で変更された Value を決定的に伝播させるためのものである。
MCP / LLM の完了、timer、承認の到着、webhook、timeout、cancellation、budget の枯渇は runtime のイベントとして
流入し、保持された Value の polling や delta cycle の再実行によって扱うのではない。永続 store の API は変更通知を
同じ順序付きのイベント履歴に種別を分けて永続化する。webhook と承認到着のイベント provider、および永続 store を
使うイベント購読は将来の adapter とする。

## 永続的な配送の境界

`RunStore` は、遷移を公開する前、または device を poll する前に、複製した遷移を write / sync / rename /
ディレクトリの sync で永続化する。これには Value の代入、イベントの publish、invocation の終端状態、イベントの消費が
含まれる。書き込みに失敗した store はそれ以降の遷移を受け付けない。書き込みが確定したかもしれない後に、古いメモリ上の
スナップショットを使ってはならない。復旧には永続化された checkpoint を調べる。`ExecutionState` のメソッド単体は
永続化を約束しない。

配送は、1つの runtime 所有者のもとで最新の checkpoint から復元された、保持済みのイベント ID について at-most-once
である。消費は payload を返す前に行う。その後のクラッシュでは handler の実行が失われうる。外部副作用の
exactly-once 保証や自動再発行はない。外部書き込みの不確定性は下記の Effect journal が別に記録する。消費済みの ID と
イベント履歴はガベージコレクションせずに保持する。store は現在スナップショット全体を書き直す。これは単一ホストの
prototype であり、上限付きのキューや複数プロセスの並行 journal ではない。

runtime の checkpoint は Value / Event / 結果の payload を含むため、ワークフローのデータと同じ機密性を保たなければ
ならない。監査 JSONL には payload を追加しない。checkpoint の検証は、機密区分・認可・capability への準拠を
示すものではない。

## Effect、retry、idempotency（Phase 3）

書き込みは独立した `Effect` であり、正規化した記述（operation、action、resource、class、JSON 引数の SHA-256）に
結び付く。runtime が `effect_id` と安定した idempotency key を所有する。route は、provider が対応する MCP 引数名を
key のために指定できる。planner の入力はそれを選んだり上書きしたりできない。これは信頼された設定上の主張であり、
provider の実際の動作の証明ではない。

Effect の状態機械:

```text
NOT_STARTED --durable start before dispatch--> STARTED
STARTED --confirmed response-------------> CONFIRMED
STARTED --error/timeout (remote write)---> UNCERTAIN
STARTED --reported error (LOCAL_WRITE)---> FAILED
STARTED --crash/reopen-------------------> UNCERTAIN
UNCERTAIN --confirmed evidence-----------> CONFIRMED
UNCERTAIN --proven not_found-------------> FAILED
UNCERTAIN --still uncertain-------------> UNCERTAIN
FAILED --new attempt, same descriptor----> STARTED (non-destructive only)
```

最初の永続化で `NOT_STARTED` を予約する。次の永続化で、device を poll する前に pending の Invocation とともに
`STARTED` を確定する。正規化に成功した応答は、invocation の結果と Effect の確定を1つのスナップショットで確定する。
`EXTERNAL_WRITE` と `DESTRUCTIVE` では、dispatch 後のエラーは timeout や意味上の失敗に見えても `UNCERTAIN` とする。
`LOCAL_WRITE` では報告されたエラーを `FAILED` とする。対象はオーケストレータ自身の環境内にあるため、返された
エラーを確定情報として扱い、再試行を許す。transport の成功だけでは不十分であり、書き込みの確定には adapter の
正規化の成功が必要である。クラッシュ後、ローカルの確定前に provider が成功していた場合は、run を再度開いたときに
`UNCERTAIN` として表す。

`UNCERTAIN` は自動的に再試行しない。信頼された照合処理が provider の証拠を調べ、confirmed、not_found、
still_uncertain のいずれかを報告する。not_found が証明された場合は、非 destructive の Effect を同じ key で再試行
できる。destructive の Effect は決して自動再試行しない。確定済みの重複要求は、provider を再度呼ばずに保存された
Effect の結果を返す。Effect は正確な1つの action instance（operation、action、resource、payload hash）である。
同じ operation で payload や宛先が変われば、新しい key を持つ新しい Effect となり、古い key は再利用しない。
`EXTERNAL_WRITE` と `DESTRUCTIVE` では、同じ operation の別の Effect が `STARTED` または `UNCERTAIN` の間は、新しい
instance を予約しない。変種が未解決の書き込みを重複させうるためである。`FAILED` は provider に存在しないことが
証明された場合、または報告された `LOCAL_WRITE` のエラーを意味し、曖昧な遠隔の transport エラーを意味することはない。

`PURE` と `READ` の呼び出しは通常の retry 方針を使ってよい。`LOCAL_WRITE` は Effect journal を持ち、報告された
エラーは再試行できるが、クラッシュ後に復元された実行中の書き込みは `UNCERTAIN` であり照合を要する。
`EXTERNAL_WRITE` と `DESTRUCTIVE` は、さらに初回の dispatch の前に provider の key 引数か、明示的な手動照合の
route 設定のいずれかを必要とする。tool がエラーを返したというだけで広い retry を許すことはない。action に bind した
承認とパラメータ付き capability model は [SECURITY_SPEC](SECURITY_SPEC_ja.md) が定める。

`RunStore::open` は v5 だけを読み、所有する run ID を検証し、`STARTED` の Effect を `UNCERTAIN` に変換して
永続化する。run ディレクトリの所有者は1つであることを前提とする。照合は信頼された runtime API として提供し、
planner の JSON や provider への自動照会としては提供しない。CLI resume とプロセス間の run lock はまだない。
イベントの消費と外部 Effect の実行は1つの原子的トランザクションではない。クラッシュ後、消費済みの Event に対応する
Effect が完了していない場合があり、復旧では両方の記録を調べなければならない。metadata のみの Effect 監査は
生の引数と結果を含まない。checkpoint 自体は結果を含み、ワークフローのデータとして保護する必要がある。

## 承認と capability

action に bind した承認（Phase 4）と capability model（Phase 5）はセキュリティの意味論であり、
[SECURITY_SPEC](SECURITY_SPEC_ja.md) が定める。runtime は、そこで定める遷移でこれらを強制する。承認の消費は
`STARTED` と同時に行い、認可は MCP・model・ファイル・sandbox への各アクセスの前に行う。

## 完了（Phase 6）

planner の `complete` の判断は評価の依頼であり、完了そのものではない。runtime は信頼できる事実だけ（action の
状態機械、Effect journal、typed adapter の構造化出力）から run の完了方針を評価する。planner の JSON schema に
事実を主張するフィールドはなく、未知のフィールドは拒否する。

条件（`config/routing-defaults.toml` の workflow ごとの `[workflows.<name>.completion]`、なければ最上位の
`[completion]`）:

| 条件 | 意味 | 出自 |
|---|---|---|
| `succeeded:<action>` | 状態機械が成功を記録した | MCP route では決定的、model route では model 由来 |
| `effects_resolved` | `STARTED` / `UNCERTAIN` の Effect がない。常に暗黙の hard 条件 | 決定的 |
| `structured:<action>:<JSON pointer>=<JSON>` | その action の最新の typed adapter の `structured` の値がリテラルと等しい | 決定的 |
| `planner_claim` | planner が完了を求めた | model 由来 |

規則:

- `hard` の条件は決定的でなければならない。model 由来の条件を `hard` に置くと設定エラーとなる。`soft` には
  どの条件も置ける。
- `safety_critical = true` には少なくとも1つの明示的な hard 条件が必要である。class の上限が `external_write` または
  `destructive` の step を含む workflow は safety-critical でなければならない。したがって LLM だけによる完了は、
  外部の安全性に影響しえない workflow に限られる。
- 結果: `COMPLETE`。`INCOMPLETE`（hard 条件が未達だが到達可能。planner は未達の一覧とともに `COMPLETION_REJECTED` を
  受け取り、ループは続く）。`BLOCKED`（未解決の Effect があり、照合のために run を止める）。`FAILED`（hard の
  `succeeded` / `structured` の action が失敗し試行回数が尽きた）。`REQUIRES_REVIEW`（hard は満たし、soft が未達で、
  既定の `on_soft_failure = "requires_review"` の場合）。`on_soft_failure` は `retry`（`INCOMPLETE` として扱う）または
  `complete` にもできる。
- `REQUIRES_REVIEW` は回答の先頭に `[REQUIRES_REVIEW]` と未達の soft 条件を付けて返し、run の phase を
  `requires_review` とする。
- すべての評価を `completion_evaluated` として監査し、`run_completed` に最終状態を記録する。拒否された完了の
  繰り返しは iteration budget が制限する。

最終回答の文章は依然としてモデルの生成物である。完了は必要な決定的 step が実行されたことを保証するが、文章が
それを正しく記述していることは保証しない。決定的なレポート生成は今後の課題とする。

## AWHDL 設計の実行

`aiconductor run-design <file> --input name=value` は設計をコンパイルして実行する。

コンパイルは構文解析と静的検査を行い（診断が1つでもあれば失敗）、各 device を `generic (route => "...")` で
設定済みの route に束縛する。

- 種類: `agent` は model route または Codex route に、`mcp` は Codex 以外の MCP route に、`deterministic` は
  class が書き込みでない Codex 以外の MCP route に束縛する。その他の種類は拒否する。
- device の宣言した location（既定 `local`）は route の `location`（既定 `local`）と一致しなければならない。
  これにより、静的なフロー検査が実際の宛先について判断する。Codex の route は `cloud` である。
- planner の route は束縛できない。設計には architecture がちょうど1つ必要である。
- budget の `iterations`、`wall_time`、`tool_calls`、`model_calls` を強制し、設定済みの上限を下げることしかできない。
  それ以外の項目名はコンパイルエラーとする。timer には `iterations` または `wall_time` の上限が必要である。

実行は delta cycle による:

1. 入力ポートに値を代入し、`name` と `name.changed` のイベントを発生させる。
2. 各 delta で、感度がその delta のイベントを含む process をソースの順にすべて実行する。process は delta 開始時の
   値を読む。
3. 代入と device の結果は delta の終わりに反映する。1つの delta で2つの process が1つの信号に異なる値を書くことは
   エラーとする。値が変われば `name` / `name.changed` を発生させる。barrier は、前回の発火以降にすべてのメンバーが
   書き込まれたとき `barrier.ready` を発生させる。
4. 並行 `assert always/never` は各 delta の後に評価し、違反すれば run を失敗させる。逐次の `assert` は直ちに run を
   失敗させる。
5. 保留中のイベントがなければ次の timer の tick を待つ。timer がなければ run は `quiescent` で終わる。budget に
   達すると run は `exhausted` で終わる。

device の呼び出しは、束縛した route を engine の通常の経路（capability の認可、Effect journal、承認、監査）で
実行する。成功すると `device.done` を発生させて出力を書き込む。失敗すると `device.failed` を発生させ、何も
書き込まない。呼び出しの `timeout` は `device.timeout` も発生させる。process の `timeout` は本体の書き込みを破棄し、
`on timeout` を実行する。`parallel` の結果はまとめて反映するが、v0.1 は呼び出しを1つずつ順に行う。run は
すべての Effect が解決したときに限り完了する。信号の値はメモリ上に保持し、監査には信号の名前と区分だけを記録し、
値は記録しない。

`x <= declassify e using d;` は、`e` を評価し、`d` の手段を順に行う（決定的な検査、次に人の承認。SECURITY_SPEC の
機密解除）。すべてが認めたときだけ `x` に書き込み（区分は `d` の `to`）、`d.done` を発生させる。1つでも認めなければ
何も書き込まず、`d.failed` を発生させる。検査の呼び出しは通常の device 呼び出しと同じ経路を通る。
