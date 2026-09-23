# Security 仕様

日本語（正本） | [English reference translation](SECURITY_SPEC.md)

> **正本の範囲。** 規範文書: [LANGUAGE_SPEC](LANGUAGE_SPEC_ja.md)、
> [RUNTIME_SPEC](RUNTIME_SPEC_ja.md)、[SECURITY_SPEC](SECURITY_SPEC_ja.md)、
> [IR_SPEC](IR_SPEC_ja.md)、[PROFILE_v0.1](PROFILE_v0.1_ja.md)。非規範:
> [IMPLEMENTATION_GUIDE](IMPLEMENTATION_GUIDE_ja.md)、例、チュートリアル、移行メモ、状態報告。
> 矛盾時は PROFILE_v0.1、次に各規範文書が優先する。v0.1 の実装範囲は PROFILE_v0.1 だけが定める。
> 日本語版を正本とし、英語版は参考訳とする。差異がある場合は日本語版が優先する。

範囲: 信頼の原則、機密区分、taint、cloud egress、機密解除（declassification）、capability、人間の承認、監査。
各項目が v0.1 で必須かどうかと実装状態は、[PROFILE_v0.1](PROFILE_v0.1_ja.md) だけが記録する。

## 信頼の原則

- LLM と agent を信頼された control plane にしない。policy、スケジューリング、sandbox、決定的な検査、
  budget、人間の承認を信頼側に置く。
- agent は自身の identity、機密区分、capability、承認、Effect の identity、完了状態を選べない。それを試みる
  planner の出力は拒否する（runtime の閉じた判断 schema を参照）。
- 許可されていないものはすべて拒否する。

## 機密区分、taint、cloud egress、機密解除

目標とする意味論は AI Conductor システム仕様
[`AI_CONDUCTOR_SYSTEM_SPEC_v0.2_ja.md`](AI_CONDUCTOR_SYSTEM_SPEC_v0.2_ja.md)（日本語が正本、英語の参考訳あり）の
5節（信頼 zone）から11節（多層防御）が定め、これらの主題について規範とする。上位プロジェクトの
`SECURE_ORCHESTRATOR_SPEC_v0.1.md` は歴史的な設計資料である。

v0.1 について PROFILE_v0.1 が要求するのは、機密区分 `public`・`internal`・`restricted`、location `local`・
`cloud`、および `restricted` のデータを `cloud` の location へ決して流さない静的規則である。declassifier、
制御フローを通じた taint、cloud DLP / egress 検査は v0.2 とする。

### v0.1 のフロー規則（実装済み）

- 値の区分は宣言したラベルである。ラベルのない型は `restricted` とする。式の区分は、読む信号のうち最も強い区分で
  ある。リテラルと、device・timer・barrier のイベントは `public` とする。
- `AWHDL-E301`: cloud の下限以上のデータを、location が `cloud` の device の引数にしてはならない。下限は
  `restricted` であり、`assert never (<class> -> cloud);` で引き下げられる。
- `AWHDL-E302`: 引数の区分は、device が宣言した `clearance` を超えてはならない。
- `AWHDL-E303`: device の結果は引数の最も強い区分を保ち、代入は値の区分を保つ。いずれかをより低いラベルの信号に
  格納することは暗黙の機密解除であり拒否する。これがなければ、ローカルの device を通してデータを低いラベルに
  洗浄できてしまう。
- `location` のない device は `local` とする。束縛時に、local の device が cloud の route に束縛されることを拒否する。
  route の location は設定（`location = "cloud"`）で与える。
- 実行時にも、各 device 呼び出しの前に、引数の宣言区分から同じ cloud 下限と clearance の検査を再び行う（多層防御）。
- v0.1 では扱わないもの: `if` の条件を通じた暗黙のフロー、宣言ラベルを超えた process 間の taint（各格納はその
  ラベルに対して検査する）。

## Action に bind した承認

承認は抽象的な真偽値ではない。1つの generation における正確な1つの Effect instance を認可する。runtime は Effect の
記録と現在の scope から正規化した action 記述を作り（[IR_SPEC](IR_SPEC_ja.md#承認v4) を参照）、承認をその SHA-256
である `action_hash` に bind する。payload、宛先 / resource、commit や diff（いずれも payload）、generation、
capability、class のどれかが変われば hash が変わるか別の Effect となり、以前の承認は適用されない。

```text
reserve Effect (NOT_STARTED)
  -> request_approval(effect, exact parameters)   -> PENDING
  -> Human device shows display text + canonical action
  -> decide_approval(id, echoed action_hash)      -> GRANTED / DENIED
  -> begin + start_effect: recompute hash, check expiry,
     GRANTED -> CONSUMED and Effect -> STARTED in one checkpoint
```

- `request_approval` は正確な dispatch の引数を必要とし、その hash は Effect の content hash と一致しなければならない。
  Human への依頼は、根拠にならない表示用の `display` と、承認対象である `canonical_action`（記述と引数）の両方を
  持つ。表示文だけを承認の根拠にしてはならない。
- 判断は依頼の `action_hash` を echo し、`expires_at` までに届き、Effect がまだ現在のものでなければならない。
  呼び出せるのは信頼された adapter のコードだけであり、planner からの経路はない。
- 承認はちょうど1回の dispatch で消費する。`not_found` の照合後の再試行を含め、再度の dispatch には新しい承認が
  必要である（replay 防止）。Effect ごとに有効な（pending または granted で期限内の）承認は最大1つとする。
- 新しい generation では以前の Effect が古くなるため、その承認は判断も消費もできない。新しい generation の同じ
  action は新しい Effect である。
- scope の語彙は `single_action`、`transaction`、`time_limited_session`。本 runtime は `single_action` だけを
  受け付け、広い scope は policy により許可されないものとして拒否する。
- route の設定 `human_approval`（`required` / `not_required`）の既定は、`external_write` と `destructive` では
  必須、それ以外では不要である。destructive の route は除外できず、書き込みでない route はこの設定を持てない。
  これは Codex 固有の `approval_policy` とは別のものである。
- 監査 JSONL は `approval_requested`（ID、hash、期限）、`approval_decided`（状態）、および `effect_started` に
  消費した `approval_id` を記録する。引数は監査に書かない。

### 人間の承認 adapter

`config/runtime.toml` の `[approval]` は承認用の MCP server（承認者モードの HumanPort、
`scripts/mcp/start-humanport-approver`）を指定する。承認が必要な書き込みでは、engine は Effect を予約し、正確な
dispatch の引数で承認を作り、人間に問い合わせる。HumanPort の task は正規化した action と `action_hash` を持ち、
回答された task はその hash を echo しなければならない。判断は dispatch の前に記録する。拒否、hash の不一致、
期限切れ、adapter のエラーのいずれでも、Effect は `NOT_STARTED` のままで provider は呼ばれない。

- 承認者モードでは `human.answer`、`human.cancel`、`human.list` を MCP から除き、回答は GUI だけが行う。
  adapter は回答手段を公開したままの server を拒否する。
- どの route も承認用の server を使えない（設定の検証）。したがって planner も agent の route もそこへ到達できない。
- `[approval]` がなければ、承認が必要な書き込みは fail closed となる。

## Capability model

tool、ファイル、sandbox、network、model の権限は1つの model であり、`config/capabilities.toml` で宣言する。

```text
Capability { id, subjects, action, resource = kind:pattern, constraints, effect_class, revoked }
authorize(subject, CapabilityRequest { action, resource, params }) -> { id, handle, effect_class }
```

- 既定は拒否。要求が許可されるのは、subject（route 名）が保持する取り消されていない grant が、同じ action、
  一致する resource パターン（`**` は複数の区切りにまたがり、`*` は1つの区切り内）、すべての要求パラメータを
  含みかつすべてが与えられている constraints を持つ場合に限る。列挙されていないパラメータは拒否する。
- resource: `mcp:<server>/<tool>`、`file:<プロジェクト相対パス>`、`model:<profile>`、`host:<name>`、および
  `repo:<name>` のような領域別の種類。`file:` の resource はパスを正規化し symlink を解決して作る。プロジェクト
  root の外のパスは照合の前に拒否する。
- 複数の grant が一致した場合は最も重い `effect_class` を採る。広く重なる grant によってリスクが過小評価される
  ことを防ぐためである。同順位は設定の固定順で決まる。
- 認可した grant の `effect_class` が、その呼び出しの Effect journal（Phase 3）と承認の既定（Phase 4）を決める。
  したがって class は route 単位ではなく tool 単位である。route の `effect_class` は上限であり、それを超える
  grant は設定の検証で失敗する。これにより route 単位の idempotency / 照合 / 承認の検査が健全に保たれる。
- handle（`id#fingerprint`）は grant の範囲が変わるたびに変わり、Effect と承認はそれに bind する
  （[IR_SPEC](IR_SPEC_ja.md#capability-の-bindv5) を参照）。
- `Capability::narrow` は部分 capability を導く。同じ action、親の範囲内の resource（保守的な包含判定）、
  同じ constraint のキー（各値は等しいか、親のパターンに一致するリテラル）、上がらない class が条件である。
  取り消された grant は拒否し、narrow もできない。

engine（planner による run と設計の run）での強制箇所:

| アクセス | 要求 | 箇所 |
|---|---|---|
| MCP tool | `mcp:<server>/<tool>` への `mcp.call` | adapter の準備後、dispatch の前 |
| planner が渡すファイル引数（Lean、Prolog、MATLAB のファイル tool） | `file:<path>` への `file.read` | adapter の準備 |
| Codex の sandbox | `host:local` への `sandbox.read_only` / `sandbox.workspace_write` / `sandbox.full_access` | Codex adapter |
| Codex の network | `host:*` への `network.connect`（full access は常にこれを含む） | Codex adapter |
| model | 各候補と planner の `model:<profile>` への `model.chat` | model の起動前 |

設定の読み込みでは、すべての route の静的に必要な権限（tool、model、sandbox、network）を認可し、未知の route を
名指す grant と、route の class を超える grant を拒否する。`mcp.call` の認可は handle と class とともに
`capability_authorized` として監査する。拒否は `capability denied` のエラーで action を失敗させる。agent は
credential を受け取らない。SSH の鍵と CLI のログインは起動スクリプトの側にとどまり、route は handle だけを持つ。

扱わないもの: 概念上の `authorize(subject, capability, resource, classification, location, context)` のうち
データの区分と location の引数（cloud-export gateway は未実装）、passthrough tool（KDB、filter）の引数の中身
（各 MCP server 自身の root 制限に依存）、Codex 内のホスト単位の network 制御（全か無かしかできない）、
credential broker。

## 監査

- 監査 JSONL は metadata のみである: 識別子、operation と action の名前、resource、class、状態、hash、capability の
  handle、budget の使用量。生の tool 引数、承認の引数、モデルのプロンプトや結果は含まない。
- 実行 checkpoint（`execution.json`）は payload を含む。結果、Value、Event の payload、Effect の結果を保存し、
  ワークフローのデータの機密性を引き継ぐため、それに応じて保護しなければならない。
- セキュリティ上の判断を監査する: `capability_authorized`、`approval_requested`、`approval_decided`、
  `effect_started`（消費した承認を含む）、`effect_finished`、`effect_reconciled`、`completion_evaluated`。
- 監査の網羅性はそれ自体ではセキュリティ制御ではない。拒否は、それを要求した action の失敗として記録する。

## 完了の信頼境界

完了は runtime が信頼できる事実から評価し、model 由来の信号が hard 条件を満たすことは決してない。規則は
[RUNTIME_SPEC](RUNTIME_SPEC_ja.md#完了phase-6) にある。
