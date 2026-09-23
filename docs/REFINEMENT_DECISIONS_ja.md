# 仕様整理の設計判断

> **位置づけ（Phase 7）:** 非規範。規範文書は LANGUAGE_SPEC・RUNTIME_SPEC・SECURITY_SPEC・
> IR_SPEC・PROFILE_v0.1 で、v0.1 の範囲と実装状態は [PROFILE_v0.1](PROFILE_v0.1_ja.md) のみが定める。

対象: `AI_CONDUCTOR_SPEC_REFINEMENT_PROCEDURE_1-7.md`。
既存の言語設計判断は `DESIGN_DECISIONS_ja.md` を参照。

## Phase 1 — 2026-09-23

- 実装は `awhdl/crates/aiconductor-runtime` に置く。AWHDL compiler は
  Milestone 1 であり、存在しない IR compiler / signal scheduler を実装済みとは扱わない。
- 実行 IR の identity 部分を schema v1 として先に定義する。AST の JSON を
  実行 IR と呼ばず、parser/AST の既存構文は変更しない。
- 1 execution context = 1 run / correlation。複数 task は別 context とする。
  retry は同じ operation slot の attempt、論理的な改訂は generation を増やす。
  LLM がこれらの値を指定する API は設けない。
- 親を指定した並列 branch は異なる operation slot と同じ generation を使用する。
  barrier は payload の有無ではなく runtime 所有の完了記録と完全な identity を確認する。
- generation は明示的な trusted API でのみ更新。現在の serial engine は 0 のまま。
  将来の scheduler 接続と AWHDL parallel 構文は未実装である。
- checkpoint は pending を送信前に永続化し、復元時には自動再送しない。
  effect journal / UNCERTAIN の設計は Phase 3。identity の永続化だけでは
  外部副作用の exactly-once は保証できない。
- `execution.json` を最新 identity の正本とし、`state.json` のコピーとの
  原子的同時更新は要求しない。復元では run ID の一致確認が必要。
- security checker は未実装のまま。identity 検査を分類／Capability 検査と
  混同しない。audit へ追加するのは metadata のみで payload は追加しない。
- `causation_id` は Phase 2 の event_id と合わせて決定する。

## 未実施

手順書の Phase 1〜7 はすべて実施済み。以後の実装範囲は PROFILE_v0.1 に従う。

## Phase 1 検証状況

2026-09-23 の再開時に fixture と v0.2 文書の読み取りが成功した。
workspace 全33テスト（runtime 29件を含む）、fmt、workspace Clippy が成功。
test / Clippy は `--offline` で実施し、Clippy は警告をエラーとして検証した。
未完了だった文書参照を更新し、上記の実装境界内で Phase 1 を完了とする。
読み取り障害の原因は未確定。次は Phase 2 を独立した変更として扱う。
詳細は SPEC_REFINEMENT_PROGRESS.md。

## Phase 2 — 2026-09-23

- `Value<T>` / `Event<T>` / `Invocation<T>` を Rust の別型として定義し、永続化時は
  JSON payload に特殊化する。ソースの型構文・AST・compiler は拡張しない。
  既存 parser は dotted sensitivity を文字列として保持し、checker は root 名のみ検査する。
  `.changed` / `.completed` の意味解析は未実装。この境界と event 宣言の拒否をテストする。
- Value の changed 判定は JSON 値の等価性。同一世代の同値代入は通知しない。
  未定義への最初の代入は null でも変更。世代を進めると旧値を自動継承しない。
- Event は UUID で区別し、同一 payload でも別発生とする。単一の消費者を想定し、
  消費済み ID を保存する。複数購読者への fan-out / delivery ledger は未実装。
- 消費を永続化してから配送する at-most-once API を選ぶ。消費後 crash の未処理は
  起こり得るため exactly-once と呼ばず、外部副作用との原子性を Phase 3 に残す。
- Invocation 結果・終端状態・終端 Event は同じ snapshot に記録する。
  完了は Value の代入を意味しない。cancelled は provider の停止保証ではない。
- successful null を欠損と混同しないよう結果を `{payload: T}` で包む。
- `causation_id` は Event ID とし、同じ scope の既存 Event にのみ bind する。
  `parent_invocation_id` とは別概念。completion Event は呼び出しの cause を継承する。
- checkpoint v2 は payload と消費履歴を持つ。v1 からの暗黙の補完移行は拒否する。
  v1 schema は履歴として保存する。audit JSONL に今回追加する payload はない。
- `RunStore` はコピー上の遷移を永続化してから採用する。保存エラー後は処理を止め、
  古いメモリ状態から消費・dispatch を続けない。単一 owner・最新 checkpoint を前提とする。
- serial engine の全 dispatch で完了 Event を生成するが、既存の同期 return を維持。
  Event scheduler、delta-cycle 実行、CLI resume、timer 等の adapter は実装済みとしない。
- security checker は既存 structural checker のまま。checkpoint payload の保護や
  capability enforcement が新たに実装されたという主張はしない。

検証結果と共通チェックは 進捗、移行制約は
[MIGRATION_PHASE_2.md](MIGRATION_PHASE_2.md) に記録する。

## Phase 3 — 2026-09-23

- 書き込みを通常の Invocation と区別し、`Effect` として checkpoint に journal する。
  class は route 設定の `effect_class`（`pure` / `read` / `local_write` /
  `external_write` / `destructive`）。MCP の省略時は `external_write`、model は `pure`。
  class は trusted 設定上の主張であり、sandbox や provider 挙動の証明ではない。
- Effect は正確な action instance（operation・action・resource・payload hash）単位。
  payload / 宛先が変われば新しい Effect と新しい key を採番し、旧 key を再利用しない。
  同一 instance の class 変更は拒否する。
- idempotency key は `awhdl:<run UUID>:<effect UUID>` を runtime が生成する。
  provider が受け取る引数名は設定が所有し、planner が同名引数を渡すと拒否する。
- `external_write` / `destructive` は provider key 引数か `manual_reconciliation`
  のどちらかがない限り初回 dispatch も許可しない。`manual_reconciliation` は
  人間承認ではない（承認の bind は Phase 4）。
- `NOT_STARTED` を dispatch 準備前に、`STARTED` と pending Invocation を future の
  poll 前に同じ snapshot で永続化する。成功時は Invocation 完了と `CONFIRMED` を同時に記録。
- `external_write` / `destructive` のエラー・timeout・正規化失敗は `UNCERTAIN`。
  `UNCERTAIN` は key があっても自動再試行しない。同じ operation に未解決の Effect が
  ある間は、別 payload の新 instance も予約しない（変種による重複送信を防ぐ）。
- `local_write` の報告済みエラーは `FAILED` とし再試行・修正後の再実行を許す。
  MATLAB（`calculation_graphing`）と KDB（`table_analysis`）はこの class に置く。
  当初 MATLAB を `external_write` としたため、コードのエラー1回で同じ run の
  以降の MATLAB 呼び出しがすべて拒否され、planner の修正・再実行ループが止まる
  退行があった。研究室所有ホスト上の計算であり、返ったエラーは確定情報として扱う。
- crash で `STARTED` のまま残った Effect は class を問わず復元時に `UNCERTAIN` とし、
  `RunStore::open` がその遷移を他の呼び出し前に永続化する。
- trusted な `reconcile_effect` は `confirmed(result)` / `not_found` /
  `still_uncertain` を受け付ける。`not_found` 後は非 destructive のみ同じ key で再試行。
  destructive の自動再試行は禁止。provider への自動照会と CLI resume は未実装。
- 確認済みの同一 instance の再要求は provider を呼ばず保存結果を返す。
  メモリ上の `successful_mcp_calls` は最適化にすぎず、再起動を跨ぐ重複排除は journal のみ。
- v1 / v2 checkpoint は v3 へ自動移行しない。送信済みか未送信かを区別できないため、
  Effect 記録を捏造しない。audit JSONL には metadata（hash・class 等）のみを追加する。

検証結果は 進捗、移行制約は
[MIGRATION_PHASE_3.md](MIGRATION_PHASE_3.md) に記録する。

## Phase 4 — 2026-09-23

- 承認の bind 先は Phase 3 の Effect instance とする。承認用に別の action 同定を
  作らず、runtime が Effect 記録と現在の scope から正規化記述を再構成して hash する。
  記述は run・correlation・generation・effect_id・operation・action・capability・
  resource・class・content hash。payload／宛先／commit／diff／generation／
  権限範囲／destructive level の変更はいずれも hash が変わるか別 Effect になる。
- 承認依頼は正確な dispatch 引数を必須とし、Effect の content hash と照合する。
  Human へは表示用 `display` と承認対象 `canonical_action`（記述＋引数）を分けて渡す。
- 判断は依頼時の `action_hash` の echo を必須とする（adapter の取り違え防止）。
  判断時・消費時ともに hash を再計算し、有効期限を確認する。
- 承認は1回限り。`STARTED` と同じ遷移で消費し、`consumed_by` に invocation を記録。
  `not_found` 後の再試行を含む再 dispatch は新たな承認が必要（replay 防止）。
  Effect ごとに有効な承認（pending / granted・期限内）は最大1件。
- 有効期限は必須で最大24時間。時刻は orchestrator の wall clock。
- scope は `single_action` / `transaction` / `time_limited_session` を型として定義し、
  runtime は `single_action` のみ受け付ける。広い scope は実装しない。
- route 設定名は `human_approval`（`required` / `not_required`）。Codex CLI へ渡す
  既存の `approval_policy` と混同しないため別名とした。既定は external_write /
  destructive で必須。destructive は除外不可。非書き込み route には設定不可。
- 承認 adapter は未接続のため承認必須 action は fail closed。ユーザー判断により
  `open_data_acquisition` だけ `not_required` を明示し現状動作を維持する。
- HumanPort は `human.answer` を同じ MCP 面に公開しており、agent が自分の承認に
  回答できるため、そのままでは承認者として信頼しない。分離は後続作業。
- 正規化は serde_json（key 整列）であり RFC 8785 とは主張しない。
  capability は Phase 5 までの暫定値 `<class>:<resource>`。
- checkpoint v4。v3 以前は承認記録を捏造できないため自動移行しない。
  audit には ID・hash・状態のみ記録し、引数は記録しない。

検証結果は 進捗、移行制約は
[MIGRATION_PHASE_4.md](MIGRATION_PHASE_4.md) に記録する。

## Phase 5 — 2026-09-23

- 権限を `Capability { id, subjects, action, resource(kind:pattern), constraints,
  effect_class, revoked }` の1種類に統一し、`config/capabilities.toml` に置く。
  問い合わせは `authorize(subject, request)` の1本。既定は拒否。
- MCP tool（`mcp.call`）・ファイル（`file.read`）・Codex sandbox（`sandbox.*`）・
  network（`network.connect`）・model（`model.chat`）を同じモデルで判定する。
  subject は route 名。agent は handle のみを持ち、credential は起動スクリプト側に残す。
- constraints は閉じた世界: 要求パラメータはすべて constraint に列挙され一致する必要があり、
  constraint はすべて要求に含まれる必要がある（未知の `force` 等を拒否）。
- 複数 grant が一致したら最も重い effect class を採る（広い grant によるリスクの過小評価を防ぐ）。
  このため KDB などは tool ごとに重ならない grant を書く。
- effect class の正本は capability とし、tool 単位で journal・retry・承認の既定を決める。
  route の `effect_class` は上限（ceiling）として残し、上限を超える grant は設定エラー。
  route 単位の idempotency / manual reconciliation / 承認除外の検査はこの上限で行う。
- file resource は canonicalize（symlink 解決）後、プロジェクト root 外を拒否し、
  root 相対パスでパターン照合する。許可は `examples/**` と `.runtime/` 配下の4領域のみ。
  `.runtime/secrets` と `config/` は付与しない。従来は runtime が任意パスを受け入れていた。
- Codex の `danger-full-access` は network も常に使えるため、フラグに関係なく
  `network.connect host:*` を要求する。host 単位の制限は Codex でできないため、
  広い grant として明示・記録する（隠さない）。
- handle は `id#fingerprint`（action・resource・constraints・class の hash）。
  Effect と承認記述に入り、grant の範囲が変われば承認が失効する。checkpoint v5。
- narrow は保守的な包含判定（同一・親が `/**` で終わる prefix 配下・リテラルの一致）。
  正しい部分集合を拒否することはあっても、拡大は許さない。
- 設定読み込み時に全 route の静的な必要権限を認可し、不明 subject・上限超過を拒否する。
- 分類・location 付きの authorize、cloud export gateway、passthrough 引数の検査、
  credential broker は実装しない。`mcp-servers.toml` の `capability` 文字列は説明のみ。

検証結果は 進捗、移行制約は
[MIGRATION_PHASE_5.md](MIGRATION_PHASE_5.md) に記録する。

## Phase 6 — 2026-09-23

- 従来は planner の `complete` で即座に run が完了し、action を1つも実行せずに
  完了できた。今後 planner の宣言は評価の依頼にすぎず、runtime が判定する。
- 判定材料は state machine・Effect journal・typed adapter の `structured` のみ。
  planner の JSON に判定用項目はなく、未知フィールドは拒否する（上書き不可）。
- 条件は `succeeded:` / `effects_resolved` / `structured:<action>:<pointer>=<json>` /
  `planner_claim`。model route の `succeeded` と `planner_claim` は model 由来とし、
  hard に置くと設定エラー。soft には置ける。
- `effects_resolved` は常に暗黙の hard 条件。未解決の書き込みを残して完了しない。
- 結果は COMPLETE / INCOMPLETE / BLOCKED / FAILED / REQUIRES_REVIEW。INCOMPLETE は
  不足条件を planner に返して継続、BLOCKED は照合待ち、FAILED は必須 action の試行が
  尽きた場合。soft 不成立は既定で REQUIRES_REVIEW（失敗扱いにしない）、`retry` /
  `complete` も選べる。REQUIRES_REVIEW は回答に明示の接頭辞を付け phase に残す。
- external / destructive の step を含む workflow は safety_critical 必須で、hard 条件を
  明示する。これにより LLM のみの完了は外部安全性に影響しない run に限られる。
  workflow のない自由 run は既定で `effects_resolved` のみ（現状の動作を維持）。
- 構文より意味論を先に固定する手順書の方針に従い、AWHDL の `completion when` 構文は
  未実装とし、条件は routing 設定に置く。checkpoint 形式は変更しない（v5 のまま）。
- 最終回答の文章は依然 LLM 生成で、hard 条件はその正確さを保証しない。

検証結果は 進捗、移行制約は
[MIGRATION_PHASE_6.md](MIGRATION_PHASE_6.md) に記録する。

## Phase 7 — 2026-09-23

- 文書を役割で再編: 規範文書は LANGUAGE_SPEC・RUNTIME_SPEC・SECURITY_SPEC・IR_SPEC・
  PROFILE_v0.1、非規範は IMPLEMENTATION_GUIDE・例・移行メモ・状態報告。矛盾時は
  PROFILE_v0.1、次に各規範文書。各文書の冒頭に明記する。
- LANGUAGE_SPEC は新たに本文を書かず、既存の AWHDL 言語仕様 v0.2（日本語正本）を
  規範本文に指定する入口とした。root の v0.1 言語仕様は歴史資料。runtime 意味論は
  RUNTIME_SPEC、セキュリティは SECURITY_SPEC、形式は IR_SPEC が言語本文より優先する。
- SECURITY_SPEC を新設し、承認（Phase 4）と Capability（Phase 5）の節を RUNTIME_SPEC から
  移した。分類・taint・cloud egress・declassification はシステム仕様 v0.2 の §5〜§11 を規範本文とする。
- v0.1 の範囲は手順書の推奨 subset を基本とし、external write をすでに許可しているため
  手順書の規則どおり Effect journal・idempotency・reconciliation（trusted API）・永続
  checkpoint・action-bound approval を v0.1 に含める。基本 capability check には planner
  が渡すファイルパスの scope 制限を含める。
- 状態は PROFILE_v0.1 の表だけで管理する。`yes` の主張にはテスト名の根拠が必須、
  テストのない `partial` には説明が必須で、`profile_matrix_is_backed_by_existing_tests`
  が表とコードの対応を検査する。awhdl 側の実装状況文書（日本語版は「正本」と自称）は
  表の要約と位置づけ直した。
- 承認 adapter がないため `open_data_acquisition` の承認除外（Phase 4 のユーザー判断）は
  v0.1 の既知の逸脱として Profile に明記した。
- 構文未定の構成要素（Event 宣言、`completion when`、Effect / 承認 / capability 注釈）は
  LANGUAGE_SPEC に列挙し、名前が決まるまで parser が暫定表記を受け付けないこととした。
- Codex 等の agent には IMPLEMENTATION_GUIDE の範囲規律（Profile の v0.1 行のみ実装、
  範囲変更は先に Profile と本書）を課す。root の旧実装指示は歴史資料とし範囲記述を失効させた。
- 現状 v0.1 は未完了。if・timer・timeout・assert・分類・location・restricted→cloud
  拒否・AWHDL から runtime へのコンパイルなどが残る（Profile の表が一覧）。

## v0.1 backlog の実装 — 2026-09-23

- **構文:** 言語仕様 v0.2 の記法をそのまま v0.1 の構文に固定した（device generic、timer、
  budget、barrier、assert always/never、`assert never (区分 -> 配置)`、process timeout と
  on timeout、呼び出し timeout、if/elsif/else、parallel、式）。予約語は語境界付きで識別子に
  使えない。`case`・`await`・FSM 型などは v0.1 外として構文エラーのまま。
- **分類:** v0.1 は public < internal < restricted のみ。confidential / secret と
  sandbox 等の配置は `E401`（Profile 外）、未知の名前は `E402`。区分のない型は安全側に
  restricted とみなす。
- **情報フロー:** 仕様の「restricted→cloud 拒否」だけでは、ローカル device を通して低い
  ラベルの信号に格納する洗浄を防げないため、device 結果と代入に「入力の最強区分以上」の
  ラベルを要求する（`E303`）。これは宣言ラベルの単調性であり、条件分岐を通じた暗黙フローは
  扱わない（v0.2）。`E301`（cloud）・`E302`（clearance）と合わせ、実行時にも同じ検査を再度行う。
- **route の location:** 静的検査の前提を実態に合わせるため route に `location` を追加し、
  Codex の route（OpenAI のクラウドモデル）を `cloud` とした。device の宣言配置と route の
  配置が異なれば束縛エラー。
- **実行:** AWHDL は route に束縛した device を持つプログラムにコンパイルし、delta cycle で
  実行する。device 呼び出しは engine の既存経路（capability・Effect・承認・監査）をそのまま使う。
  代入は delta 末尾に反映、同一 delta で異なる値の複数ドライバはエラー。timer には budget を必須とし、
  runtime が強制しない budget 項目（llm_tokens 等）は黙って無視せずコンパイルエラーにする。
  `parallel` は論理的な同時性（結果の同時反映）で、呼び出しは逐次。
- **承認 adapter:** HumanPort に承認者モードを追加し、MCP からの回答手段を除去。runtime は
  回答手段を公開する server を拒否し、承認用 server を route から参照することも禁じる。
  承認依頼には正規化 action と `action_hash` を載せ、回答の hash 一致を必須とする。
  `open_data_acquisition` の承認除外（Phase 4 のユーザー判断）は、adapter 完成後に所有者の判断で
  撤廃した（2026-09-23）。以後、外部書き込み・破壊的操作の route はすべて承認必須。
- **MATLAB ファイル:** MATLAB ホストから workspace が見えないため、capability 検査済みの
  `.m` を読み込み `evaluate_matlab_code` として送る（256 KiB 上限）。テストクラスは
  コードとして送れないので `run_matlab_test_file` は非対応とし route から外した。
- **テストの教訓:** 失敗を期待するテストが別の理由（構文エラー）で通っていたため、
  失敗理由をメッセージで確認し、有効な基準ケースが成功することも併せて検証する形にした。
