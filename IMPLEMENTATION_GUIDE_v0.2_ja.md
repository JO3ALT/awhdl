# AWHDL実装ガイド v0.2
## Project: AI Conductor / AWHDL / cvim

日本語（正本） | [English reference translation](IMPLEMENTATION_GUIDE_v0.2.md)

> **対象読者:** 公開AWHDLプロトタイプの実装者。本ガイドはtool-neutralであり、特定のcoding agent、model、provider account、host構成、独自serviceを必要としません。この日本語版を正本とし、英語版は参考訳とします。

実装前に日本語版の[AWHDL言語仕様](docs/AWHDL_LANGUAGE_SPEC_v0.2_ja.md)と[AI Conductorシステム仕様](docs/AI_CONDUCTOR_SYSTEM_SPEC_v0.2_ja.md)を読み、正本として扱ってください。既存framework中心へ黙って再設計せず、初期実装は小さなnative Rust prototypeとします。

## 1. 第一目標

MVPは、小さなAWHDLの解析、typed AST、静的security flow検査、version付きJSON IR、async Rust runtime、MCP stdio、event-driven process、timer/timeout、単純parallel、反復／時間／tool call budget、local/cloud区別、restricted-to-cloud拒否、structured audit、Vim向けCLIを一貫して実現します。

さらに、複数AI・MCPの反復系を比較可能な設計として表し、実行trace、評価結果、停止理由を観測できるようにして、**ループエンジニアリングに資する実装基盤**とします。agentとMCPはdeviceであり、trusted control planeではありません。

## 2. 実装言語とlibrary

Rustを使用します。asyncは`tokio`、CLIは`clap`、serializationは`serde`/`serde_json`、parserは`chumsky`または`pest`、errorは`thiserror`（必要なら`miette`）、loggingは`tracing`、一時workspaceは`tempfile`、IDは`uuid`を推奨します。依存を抑え、具体的testが要求しない限り初期版へdatabaseを導入しません。

## 3. Repository構成

AST、parser、checker、IR、runtime、MCP、policy、sandbox、CLIを責務境界として分離し、examplesとparser/security/runtime/integration testsを置きます。初期は少ないcrateでも構いませんが、module境界はこれらの責務に揃えます。

## 4. 最初はv0.1部分集合だけを実装する

対象構文はentity、architecture、device、signal、process、if、case、timer、timeout、parallel、budget、assertです。security classは`public < internal < restricted`、locationはlocal/cloud、device kindはagent/mcp/deterministic/human/declassifier、MCP transportはstdioから開始します。VHDL完全互換を目指しません。

## 5. ParserとAST

entity、architecture、device kind/location/clearance、signal type/classification、process trigger、call、assignment、if、case、parallel、timer、timeout、budget、assertionを明示的なAST型で表します。可能な限りsource spanを保持し、runtime実装前にparser testを追加します。未対応構文を部分的に解釈して成功扱いにしてはなりません。

## 6. Security lattice

再利用可能な`Classification { Public, Internal, Restricted }`と順序比較を実装します。signalは同じか上位のclearanceを持つ宛先へだけ流せます。`restricted -> internal-clearance`を拒否します。cloud locationとclearanceは別概念ですが、project/default policyでcloudへの特定区分を一律拒否できるようにします。

## 7. Taint model

派生値は入力の最大区分を継承し、通常のagent/MCP出力で区分を下げません。declassifierだけが明示的AST/IR nodeを介して低下でき、すべてのdeclassificationを記録します。初期版は高度な推論より単純で保守的な規則を優先します。

## 8. Cloud外向き通信境界

cloud device callは単一の `authorize_egress(payload_metadata, destination, classification)` 相当の抽象を必ず通します。cloud adapterによる迂回を構造的に不可能にします。初期payload検査が限定的でも、secret scan、regex DLP、構造field filter、高entropy検出、組織規則を追加できるAPI境界を設けます。

## 9. Device抽象

非同期`Device::invoke(request, context) -> Result<DeviceResponse, DeviceError>`相当のtraitを定義します。初期adapterはmock deterministic、local command/mock agent、MCP stdio、egress認可を通るmock cloudです。test suiteに実cloud資格情報を要求せず、CIではmockを使います。

## 10. MCP stdio

設定済みchild processを起動し、stdin/stdoutでprotocol通信し、timeout、exit status、process cleanup、malformed responseのstructured errorを実装します。stderrがJSON protocolを壊さないよう分離します。full MCPが大きすぎる場合もadapter interfaceと最小test server fixtureを作り、本番codeで成功を偽装してはなりません。

## 11. Runtime event model

Tokio上にevent queue/channel、signal store、timer event、process trigger、async device call、signal update、後続event生成を実装します。ready processは決定的queue順で評価し、device callは並行実行できます。厳密なsemanticsを文書化します。

## 12. Delta cycle意味論

1. eventをdequeueする。
2. readyになったprocessを評価する。
3. signal更新をstageする。
4. 更新をcommitする。
5. changed signalのeventをenqueueする。
6. 次cycleへ進む。

即時再帰的なsignal伝播を避け、単純signal chainの結果がsource順に依存しないtestを追加します。

## 13. Parallel block

`tokio::join!`または`JoinSet`等で複数deviceを並行実行し、block完了前に全結果を収集します。一分岐の失敗はstructured resultとし、orphan processを残しません。cancelとtimeoutを全分岐へ伝播します。

## 14. Timerとtimeout

周期timerとoperation/process timeoutを実装します。testでは短いms単位またはpaused Tokio timeを使い、決定的で高速にします。timeout後の遅延応答がsignalを更新しないことも検査します。

## 15. Budget

最大反復数、wall time、tool/device call数を必須とし、容易ならtokenとnetwork request countも追加します。runtimeが強制し、exhaustionをstructured event/errorとしてauditします。agentに自己申告させません。

## 16. Assertion

初期版は既知のstatic/runtime propertyに対するboolean、security-flow、retry/iteration boundを扱います。完全なtemporal assertionを実装しない場合は、明確な「未実装」診断で拒否し、黙って無視しません。

## 17. Sandbox抽象

最初のbackendがlocal temporary-directoryでもtrait/interfaceを先に定義し、rootless container、gVisor、Kata、microVMを追加可能にします。user home全体、Docker socket、SSH/cloud資格情報を既定mountしてはなりません。temporary workspaceまたはgit worktreeを使います。

## 18. Workspace動作

実repoから一時worktree/copyを作り、deviceはsandbox側だけを変更し、runtimeがdiff/resultを回収し、人間が確認後に別stepで適用します。初期版では生成patchを実working treeへ自動適用しません。

## 19. CLI

`aic check <file>`、`compile <file> -o <ir.json>`、`run <file>`、`sim <file>`を計画し、任意で`graph`を追加します。parse、type、security flow、runtime、assertion失敗にはそれぞれ非0終了状態を返し、簡潔でscript-friendlyな診断を出します。現在の実装範囲は実装状況文書に合わせます。

## 20. Vim adapter互換性

`cat selection.txt | aic run workflow.awhdl`または薄いcvim wrapperから利用できる入出力にします。最初は`--output text`と`--output json`、将来はpatch/diff modeを提供します。structured outputのschemaをversion管理します。

## 21. Audit logging

runごとにJSON Linesを出力し、run ID、timestamp、event type、device ID/kind/location、入出力classification、allow/deny、call count、budget state、error、declassificationを記録します。raw restricted payloadは既定で含めません。

## 22. 実装する例

- `hello.awhdl`: local deviceがtextを変換。
- `secure_flow_ok.awhdl`: policyが許せばinternal dataをinternal-clearance cloud mockへ送信。
- `secure_flow_denied.awhdl`: restrictedからinternal-clearance cloudへの接続を`aic check`で拒否。
- `timer_loop.awhdl`: timerで反復しbudgetで停止。
- `parallel_validation.awhdl`: 複数mock validatorを並行実行しdeterministic judgeが判定。

## 23. 次へ進む前に必要なtest

1. 最小programを解析できる。
2. syntax errorに有用な位置がある。
3. 正当security flowが通る。
4. restricted-to-cloudが失敗する。
5. classificationが保守的に伝播する。
6. declassifierだけが明示的に低下できる。
7. timerがprocessを起動する。
8. timeoutがdevice callをcancel/failする。
9. parallel deviceが時間的に重なる。
10. 最大反復でloopが止まる。
11. 最大tool callで止まる。
12. audit logを出す。
13. restricted payloadを通常auditへ書かない。
14. mock cloud adapterがegress認可を迂回できない。
15. temporary workspaceを終了後に除去する。

各milestone完了前に`cargo test --workspace`、`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`を実行します。

## 24. 実装自体のsecurity規則

model/device出力はすべてuntrustedとします。command文字列連結、untrusted値のshell補間、無検証host path、deviceによるclassification低下、cloud adapterのraw HTTP迂回、実treeへの自動適用、secret/restricted payloadの既定loggingを禁止します。subprocessにはargument arrayと明示的executable pathを使います。

## 25. 作業様式

小さなvertical sliceで進めます。

### Milestone 1

workspace skeleton、AST、parser、`aic check`、基本diagnostic。

### Milestone 2

classification lattice、signal/port、location/clearance、security tests。

### Milestone 3

JSON IR、event runtime、mock device、1 process/1 signal chain。

### Milestone 4

timer、timeout、budget、parallel。

### Milestone 5

MCP stdio、mock fixture、audit log。

### Milestone 6

sandbox抽象、temporary workspace、mock cloud＋egress gateway、denied-flow integration test。

各milestoneでtest、fmt/clippy、READMEと実装状況の更新、変更fileと制限の記録を行い、test失敗中に次へ進みません。

## 26. 推測せず文書化すべき設計問題

delta scheduling、同一signalへの複数write、cloud既定制限、declassification認可、persistent checkpointの暗号化／保存、tool schemaとAWHDL typeのmapping、async error伝播、cancel、sensitivityとexplicit event、deterministic completionの定義は、保守的に決めて`docs/DESIGN_DECISIONS_ja.md`へ記録します。実装詳細の中に隠しません。

## 27. 最初のtask

Cargo workspace、specのdocs配置、AST、hello用parser、`aic check`、parser unit test、build/test手順を持つREADMEというMilestone 1だけを実装します。cloud API、real LLM、Docker、distributed executionはまだ実装しません。完了時にfile、grammar、test、未対応構文、次stepを報告します。

## 28. 正式名称――変更しないこと

```text
AI Conductor   system全体
AWHDL          Agentic Workflow Harness Description Language
cvim           Vim向けfrontend
aic            暫定CLI executable
```

依存方向は `cvim -> AI Conductor CLI/API -> compiler/checker, runtime, policy, sandbox, MCP adapter, cloud gateway` とします。

名称のHはHardwareではなくHarnessです。実装上のharnessは、device graphだけでなく、event scheduling、情報フロー、budget、approval、completionを一体として検査・実行する境界を指します。

## 29. v0.2 repository target

rootにCargo/README/docs、`crates/`にparser/AST/checker/IR/runtime/MCP/policy/sandbox/CLI、`cvim/`に薄いfrontend文書、`examples/`と`tests/`を置きます。Milestone 1ではcvimは設計placeholderでもかまいません。

## 30. Security invariant

repository自身が権限を付与できてはなりません。project-local policyは、外側のsystem/user policyに対する実効権限を減らすことだけができます。

## 31. 第一taskの完了境界

Milestone 1のCargo workspace、AST、parser、`aic check`、source diagnostic、parser test、v0.2 docs、cvim責務文書までを完了対象とします。real cloud API、Docker、完全Vim plugin、distributed executionは対象外です。
