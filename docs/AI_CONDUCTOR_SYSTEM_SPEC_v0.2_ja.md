# AI Conductorシステム仕様 v0.2

日本語（正本） | [English reference translation](AI_CONDUCTOR_SYSTEM_SPEC_v0.2.md)

> **状態:** 公開アーキテクチャドラフト。本書は目標システムとセキュリティ境界を定義します。運用環境固有の導入手順ではなく、必須のホスト名、パス、資格情報、ベンダーアカウントを定めません。実装済みの動作は[実装状況](IMPLEMENTATION_STATUS_ja.md)を参照してください。

例ではプレースホルダーのプロジェクト、デバイス、宛先を使います。配備時のポリシーはワークフローソースの外側から与えなければならず、プロジェクト内ポリシーはシステム／ユーザーポリシーを越えて権限を拡大してはなりません。本日本語版を正本とし、英語版は参考訳とします。

## 1. 目的

AI Conductorは、AIエージェント、MCPサーバー、決定的ツール、人間、クラウドサービスを統括するlocal-firstのオーケストレーターです。主要目標は次のとおりです。

1. フィードバックを含む多段の複数AI／MCPワークフローを実行する。
2. 複数AI・MCPのループを設計、解析、シミュレーション、観測、検証、改善する**ループエンジニアリングに資する**。
3. Vimを中心とする人間の編集環境を主な操作点として保つ。
4. ローカルモデルとクラウドモデルを共存させる。
5. 指定データをローカル信頼境界の外へ出さない。
6. 外部作用を明示し、監査可能にする。
7. 特定のAI／agentベンダーに依存しない。
8. AWHDLを優先的なワークフロー記述フロントエンドとする。

```text
Vim / CLI -> cvim -> AI Conductor
                       + policy engine / scheduler / state / audit
                       + sandbox -> local LLM / MCP / compiler / prover
                       ` cloud gateway -> allowed cloud services
```

## 2. 上位設計原則

LLMを信頼済み制御プレーンにしてはなりません。信頼済み制御プレーンは、policy engine、scheduler/runtime、sandbox境界、情報フロー検査、決定的完了検査、実行予算、人間承認gateで構成します。agentは信頼されない、または部分的にのみ信頼される計算deviceです。

## 3. Vim統合

cvimは標準入力またはファイルを読み、ワークフローを選択し、ポリシーで許可された文脈を付け、runtimeを呼び、結果・patch・診断を返す薄いadapterとします。ワークフロー論理や信頼済みポリシー判断をcvim内に実装してはなりません。

## 4. ランタイム層

### 4.1 Frontend層

AWHDL、CLI、Vim選択範囲、project設定、debug用IRを入力し、IR、診断、patch、結果、承認要求、監査参照を出力します。

### 4.2 Compiler／policy解析層

AWHDLの解析、型／port、情報フロー、capability、budget、無制限cycle、cloud exportの検査を行い、実行可能なバージョン付きIRを生成します。

### 4.3 Event runtime

event queue、process／timer scheduling、非同期device call、barrier、retry、timeout、persistent state、checkpoint、決定的signal更新を担当します。

### 4.4 Device adapter

標準device classは `agent`、`mcp`、`deterministic`、`human`、`declassifier`、`external_service` です。adapterは共通の要求、応答、cancel、分類、監査interfaceへ変換します。

### 4.5 Sandbox manager

一時的な隔離環境を生成し、許可データだけをmountし、network/filesystem policyとCPU、RAM、process、時間上限を適用します。終了後に破棄し、宣言された出力だけを回収します。

### 4.6 Cloud gateway

public cloudへworkflow payloadを送信できる唯一のcomponentです。宛先allowlist、classification／taint、DLP／secret／内容検査、size/token上限、redaction、監査を担当します。一般MCP workerの直接internet接続は、明示的に必要な場合を除き拒否します。

## 5. 信頼zone

### 5.1 Local restricted zone

`public`から`restricted`、必要に応じ`secret`まで処理します。local LLM、MCP、compiler、test、simulation、proof、anonymizer等を配置します。

### 5.2 Cloud zone

project policyが許可する区分だけを処理します。cloud deviceのclearanceが`internal`なら、`confidential`以上を直接送信してはなりません。

### 5.3 Human approval zone

実working treeへのpatch適用、push、message送信、deploy、破壊操作、policyが要求するdeclassificationを扱います。承認は対象内容に束縛しなければなりません。

## 6. Project data policy

path単位でcloud許可／local onlyを宣言します。ただしpath規則だけでは、コピーや生成物による流出を防げないため不十分です。外側policyとの共通部分だけを実効許可とします。

```yaml
data_policy:
  cloud_allowed: [src/**, docs/**, data/public/**]
  local_only: [data/raw/**, secrets/**, "**/*.key"]
```

## 7. 構造化データポリシー

列単位のdeny/allowをサポートします。raw dataは、信頼されたlocal変換、検証済み派生data、cloud gatewayの順で処理します。単に列名を変えたり別形式へコピーしても制限は消えません。

## 8. Classificationとtaint

区分は `public < internal < confidential < restricted < secret` です。派生値は最強入力区分を継承し、path変更では区分は変わりません。LLM出力は使用入力すべてにtaintされ、明示的に承認されたdeclassifierだけが区分を下げられます。

## 9. Declassification

declassifierは信頼されたlocal componentです。列削除、集約、k匿名性検査、仮名化、identifier削除、規則化した統計要約等を行います。手法、入力、出力、判定、run IDを記録します。LLM自身に区分低下を許可してはなりません。

## 10. Cloud gateway

全public-cloud LLM/API callを一つのgatewayへ集約します。secret key、private key、個人情報、禁止pattern、高entropy secret、禁止列、restricted内容の複製、payload上限超過を検出し、宛先と実際のpayloadを送信直前に検査します。

## 11. 多層防御

### Level A: ソース側policy

workflowはrestricted dataをcloud-capable deviceへ露出させないよう静的に検査します。

### Level B: 外向き通信検査

gatewayはpathや生成元に関係なく実payloadを検査します。静的検査の成功は実行時検査を省略する根拠になりません。

## 12. MCP capability model

各MCP toolに `SAFE`、`READ`、`WRITE`、`EXTERNAL`、`DESTRUCTIVE` の作用classと、具体的な資源範囲を割り当てます。自動loopは通常SAFE、READ、sandbox内WRITEだけを許可し、EXTERNALとDESTRUCTIVEはpolicy、通常は人間承認を必要とします。MCP接続自体をsecurity boundaryと見なしてはなりません。

## 13. Sandbox model

既定はrootless container、read-only base、非privileged、host PID/networkなし、Docker socketなし、Linux capability drop、seccomp、可能ならAppArmor/SELinux、資源上限、制御された一時領域です。未知の第三者codeにはgVisor、Kata、microVM、専用VM等の強い隔離を選択します。

## 14. Workspace model

agentが既定で実repositoryを直接変更してはなりません。git worktreeまたは一時copyをsandboxへ渡し、変更からdiffを生成し、orchestratorと人間が確認した後に実treeへ適用します。

## 15. Secret処理

可能な限りagent-visibleな環境変数へsecretを置かず、credential brokerを持つMCP/service adapter経由で能力だけを提供します。SSH、cloud credential store、Docker socket、host-wide設定を広く公開してはなりません。audit logにもsecret値を記録しません。

## 16. Network policy

既定はDENYです。deviceごとに必要な宛先だけを許可し、可能なら監査proxyを通します。名前解決、redirect、proxy、subprocessを含めてpolicyを強制します。

## 17. 実行予算

自律loopには、最大反復、wall time、LLM token、tool call、network request、disk write等の機械的上限を設定します。上限到達はeventであり、model判断ではありません。複数予算が適用される場合は最も厳しい上限を使います。

## 18. 完了と判定

agentの「完了」という発言を完了条件にしてはなりません。build、test、lint、proof、policy check等をdeterministic judgeで統合し、失敗なら診断をloopへ戻し、成功なら完了させます。LLM reviewは追加signalにできますが、利用可能な決定的検査を優先します。

## 19. Event model

runtimeはevent queueを使用します。signal更新、MCP完了、LLM応答、file変更、timer、timeout、人間承認、budget exhausted、sandbox failure等を構造化eventとして扱います。

## 20. Checkpoint

長時間workflow、承認待ち、process restart、debug、auditのためpersistent stateをcheckpointします。secretや許可されないraw dataを信頼zone外へ暗黙に永続化してはなりません。再開時はIR、policy、device configurationの互換性を検査します。

## 21. Audit log

workflow/run ID、device call、時刻、入出力区分、宛先、policy判断、cloud exportのhash/metadata、承認、budget消費、sandbox lifecycle、error/retryを記録します。通常logへrestricted payloadを既定で書かず、metadata-onlyまたはredacted形式を提供します。

## 22. Backend非依存性

native Rust/Tokio、graph framework、agent framework、将来のdistributed runtime等を実行backendにできます。AWHDLが人向けfrontendであり、backend frameworkは実装targetです。backend差によってsecurity semanticsを弱めてはなりません。

## 23. 内部IR案

安定したversion付きJSON IRを使います。`ir_version`、workflow、devices、signals、processes、policies、budgets、assertions、source spanを保持します。未知の必須field/versionは拒否し、意味を落として実行してはなりません。

## 24. 初期CLI

計画するcommandは `aic check`、`compile`、`sim`、`run`、`graph`、`audit` です。cvimはこれらを起動し、diagnostic、diff、approval、run状態、cancelをUIへ橋渡しします。実装済み範囲は実装状況文書に従います。

## 25. Error category

`E1xx` parser、`E2xx` type/port、`E3xx` security/data-flow、`E4xx` capability、`E5xx` timing/event、`E6xx` budget/resource、`E7xx` backend/runtime、`E8xx` sandbox、`E9xx` assertion/testbenchとします。診断にはcode、source位置、関係する区分／device／policy、修正可能な説明を含めます。

## 26. MVP architecture

v0.1はAWHDL parser、AST、type/security checker、JSON IR、single-host Rust/Tokio runtime、MCP stdio、local command/agent adapter、timer scheduler、policy engine、sandbox抽象から始めます。最初からKubernetesやdistributed executionを導入しません。

## 27. 推奨repository構成

parser、AST、checker、IR、runtime、MCP、policy、sandbox、CLIを独立crateに分け、`examples`とparser/security/runtime/integration testを置きます。仕様文書と実装状況をcodeと同じrepositoryでversion管理します。

## 28. v0.1の非目標

graphical editor、distributed scheduler、Kubernetes operator、完全な形式検証、VHDL完全互換、任意embedded scripting、本番級DLP、vendor固有rich IDE、無制限shell agentは初期対象外です。小さく試験可能で安全なlanguage/runtime coreを優先します。

## 29. Prototype成功条件

小さなAWHDLの解析とtyped AST、不正port接続とrestricted-to-cloud flowの拒否、local MCP、timer、上限付きretry、並列validation、timeout/budget停止、audit、cloud adapterへのrestricted data遮断、Vim/CLIへのpatch/result返却を一貫して実証できることです。

## 30. 正式名称

| 名称 | 役割 |
| --- | --- |
| AI Conductor | orchestration system全体 |
| AWHDL | Agentic Workflow Harness Description Language。AI・MCPループのharnessを記述する言語 |
| cvim | Vim向けfrontend |
| AI Conductor IR | 検査済み内部表現 |
| AI Conductor Runtime | native event-driven runtime |

cvimはruntimeではなく、AWHDLはruntimeではなく、MCPはsecurity boundaryではなく、LLMはtrusted control planeではありません。

AWHDLのHはHardwareではなくHarnessです。ここでharnessはdeviceの接続だけでなく、情報フロー、event、反復、予算、承認、安全境界を束ねる制御構造を意味します。VHDL由来の設計概念は採用しますが、hardwareを記述または合成する名称ではありません。

## 31. 改訂architecture

`Human -> Vim -> cvim -> AI Conductor` を入口とし、内部にAWHDL compiler、static checker、IR、event runtime、policy engine、sandbox、local device/MCP、cloud gateway、human approvalを配置します。

## 32. cvimの責務

buffer／選択／fileの送信、workflow選択、check/sim/run起動、diagnostic／diff表示、approval要求、run参照、cancelを行えます。data export判断、無制限cloud資格情報の保持、gateway迂回、trusted sandbox/security logicの実装をしてはなりません。

## 33. Project-local設定

`.conductor/policy.yaml`、`devices.yaml`、`workflows/*.awhdl`を推奨します。project policyは外側のsystem/user policyより権限を減らせますが、増やせません。環境固有endpointやcredential参照はworkflowから分離します。

## 34. Policy優先順位

既定拒否とし、実効権限はsystem、user、project、workflow、device permissionの共通部分です。適用可能な明示denyが一つでもあればdenyを優先します。

## 35. 二段階cloud保護

第1段階はAWHDLの静的情報フロー検査、第2段階はgatewayによる実payload、taint、destination、DLP、secretの実行時検査です。両方を必須とします。

## 36. Data provenance

可能な限り各値にclassification、source、transform chain、declassification record、run IDを付けます。初期実装はsignal/value単位から始めてかまいません。provenance欠落時は保守的に強い区分として扱います。

## 37. Cancelと障害封じ込め

cancelは新規scheduleを停止し、cancel可能なcallとsandbox childを終了し、最小auditを保存します。未承認変更を実treeへ適用してはなりません。隔離可能なdevice failureはorchestrator全体をcrashさせず、構造化runtime eventへ変換します。

## 38. v0.2 MVP境界

必須対象はparser/AST、静的classification検査、local/cloud device、JSON IR、Tokio event runtime、MCP stdio、timer/timeout、bounded loop、parallel validation、audit、一時workspace、cloud-egress authorization、cvimから利用可能なCLIです。full DLP、distributed runtime、Kubernetes、GUI、完全な時相論理、本番microVM、rich cvim UIは延期します。
