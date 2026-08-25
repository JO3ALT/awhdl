# AWHDL言語仕様 v0.2
## Agentic Workflow Harness Description Language

日本語（正本） | [English reference translation](AWHDL_LANGUAGE_SPEC_v0.2.md)

> **状態:** 公開ドラフトであり、最終標準ではありません。本書はv0.2までに意図する言語を定義しますが、全構文の実装完了を表すものではありません。現在の適合範囲は[実装状況](IMPLEMENTATION_STATUS_ja.md)を参照してください。例中の製品、プロバイダー、ホスト、デバイス名は説明用であり、規範的ではありません。

太字の「**しなければならない**」「**してはならない**」「**することが望ましい**」「**してもよい**」は規範要件を表します。特記しない説明と例は参考情報です。この日本語版を規範文書（正本）とし、英語版は参考訳とします。差異がある場合は日本語版が優先されます。

## 1. 目的

AWHDLは **Agentic Workflow Harness Description Language（エージェント型ワークフロー・ハーネス記述言語）** の略称です。AI Conductorのワークフロー記述言語であり、AIエージェント、MCPサーバー、人間、ローカルプログラム、クラウドサービス、タイマー、データフロー、フィードバックループ、機密区分、実行制約を記述するドメイン固有言語です。

ここでHarnessとは、複数の実行主体を接続し、そのデータフロー、起動条件、反復、予算、権限、安全境界を一つの制御構造として束ねるものを指します。名称のHはHardwareを意味しません。AWHDLはハードウェア記述言語の並行性、signal、event、構造的検査という有用な考え方を借りますが、記述対象はAI・MCP・人間を含むagentic workflowです。

中心原則は、通常の逐次プログラムを書くことではなく、エージェント系の構造、接続、状態遷移、イベント反応、時間、情報フロー制約を記述することです。特に、複数AIと複数MCPを用いるループを、設計・解析・シミュレーション・観測・検証・改善できる対象とする**ループエンジニアリングに資すること**を目的とします。

AWHDLが扱う主な対象は、AIの反復修正、複数MCPの協調、ローカル／クラウドLLMの混成、人間の承認、周期・イベント駆動、並列実行、再試行とタイムアウト、機密区分、クラウドへの情報流出防止、実行予算、静的検査、シミュレーション、実行バックエンドへの合成です。

## 2. 中核言語モデル

主要構文は `entity`、`architecture`、`device`、`port`、`signal`、`process`、`timer`、`policy`、`budget`、`assert`、`configuration` です。

| AWHDL | VHDLでの類似物 | 意味 |
| --- | --- | --- |
| entity | entity | ワークフローの外部インターフェース |
| architecture | architecture | ワークフローの実装 |
| device | component/entity instance | AI、MCP、プログラム、サービス、人間 |
| port | port | 型付き入出力 |
| signal | signal | データ／イベント接続 |
| process | process | イベント駆動動作 |
| timer | clock/timing source | 時間駆動イベント |
| policy | constraint | セキュリティ／実行規則 |
| assertion | assertion | 安全性／完了の不変条件 |
| configuration | configuration | バックエンド／デバイスの束縛 |

## 3. 最小例

```vhdl
entity hello is
    port (
        task   : in  text<internal>;
        result : out text<internal>
    );
end hello;

architecture behavioral of hello is
    device llm : local_llm;
begin
    process(task)
    begin
        llm.run(task) -> result;
    end process;
end behavioral;
```

## 4. Entity

`entity`はワークフローの外部インターフェースを宣言し、実装を含みません。同一entityを複数のarchitectureで実装してもかまいません。

## 5. Architecture

`architecture 名前 of entity名 is ... begin ... end 名前;` が実装を定義します。同じ論理ワークフローに `local`、`cloud`、`hybrid` などの実装を用意し、配置を交換できます。

## 6. Device

`device`は実行コンポーネントであり、LLMエージェント、MCPサーバー、決定的プログラム、ブラウザー、外部サービス、人間、機密解除器、判定器を表します。

```vhdl
device tool : mcp
    generic (
        server    => "example-mcp",
        location  => local,
        clearance => restricted
    );
```

プロバイダー固有値はconfigurationまたはプロジェクト設定へ分離することが望ましく、言語意味を変更してはなりません。

## 7. Port

デバイスのportは方向、データ型、機密区分を持ちます。イベント用portも宣言できます。

```vhdl
port (
    request  : in  text<internal>;
    response : out text<internal>;
    done     : out event;
    error    : out event
);
```

## 8. Signal

signalはデバイスとprocessを接続し、データまたはイベントを伝達します。代入はVHDLに類似した遅延更新意味論に従います。

```vhdl
signal candidate : code<internal>;
signal result    : test_result<internal>;
```

## 9. 機密区分

組み込み区分と順序は次のとおりです。

```text
public < internal < confidential < restricted < secret
```

型は `BaseType<Classification>` と表します。資格情報自体は `secret<secret>` など、最上位区分にしなければなりません。

## 10. 情報フロー規則

値は、宛先のclearanceがその区分以上であり、かつ配置・ネットワーク・外部送信ポリシーが許可する場合だけ流せます。概念的条件は `signal.classification <= device.clearance` です。違反は静的診断とし、実行時にも外向き通信境界で再検査しなければなりません。

## 11. テイント伝播

派生データは、入力のうち最も強い機密区分を継承します。LLMが出力を安全だと主張しても区分は低下しません。

```text
A : restricted
B := summarize(A)
結果: B : restricted
```

## 12. 機密解除

機密区分を下げられるのは、明示的に信頼された `declassifier` だけです。

```vhdl
anonymous <= declassify raw using anonymizer;
```

解除器は `from`、`to`、手法、必要なパラメーターを宣言し、解除操作は監査記録に残さなければなりません。

## 13. Process

processは感度リスト内のsignalまたはeventが変化すると実行可能になります。

```vhdl
process(task, feedback)
begin
    coder.run(task, feedback) -> candidate;
end process;
```

## 14. 標準イベント

初期標準イベントは `started`、`done`、`failed`、`changed`、`timeout`、`approved`、`rejected`、`ready`、`exhausted` です。実装はイベント名と対応する実行IDを保持し、古い実行の完了を新しい実行へ誤配送してはなりません。

## 15. 時間

時間は第一級の型です。初期単位は `ms`、`sec`、`min`、`hour`、`day` とします。例: `wait for 10 sec;`。

## 16. Timer

```vhdl
timer monitor_clock : period 10 min;

process(monitor_clock)
begin
    monitor.check;
end process;
```

timerはオーケストレーション上の時刻源であり、ハードウェアクロックではありません。

## 17. Timeout

操作単位では `tool.run(input) timeout 5 min -> result;`、process単位では `timeout` と `on timeout` を宣言できます。タイムアウト時は関連処理をキャンセルし、遅れて到着した結果を現在の実行へ適用してはなりません。

## 18. 非同期デバイス呼び出し

デバイス呼び出しは既定で非同期です。要求を発行し、結果到着がsignalを更新し、それが後続processを起動します。明示的な同期には `await` を使います。

## 19. 並列実行

```vhdl
parallel
    compiler.run(candidate) -> build_result;
    tester.run(candidate)   -> test_result;
    prover.run(candidate)   -> proof_result;
end parallel;
```

分岐は論理的に並行であり、記述順を実行順保証として扱ってはなりません。

## 20. Barrier

barrierは指定した全signalについて、対象実行に対応する結果が揃ったとき `ready` を発行します。

```vhdl
barrier validation (build_result, test_result, proof_result);
```

## 21. FSM対応

列挙型signalと `case` により状態機械を表します。状態遷移はsignal更新として行い、同一デルタサイクル内の読み取りには更新前の値が見えます。

## 22. フィードバックループ

生成、試験、判定、診断付き再生成を別processとして接続できます。ループは必ず予算、期限、最大反復数、または外部停止条件の少なくとも一つで制限し、完了と失敗の経路を明示しなければなりません。

## 23. 実行予算

```vhdl
budget development_loop is
    iterations       <= 10;
    wall_time        <= 30 min;
    llm_tokens       <= 200000;
    tool_calls       <= 100;
    network_requests <= 20;
end budget;
```

上限到達時は `development_loop.exhausted` を発行します。モデルの自己申告で予算を延長してはなりません。

## 24. Retry省略記法

単純な反復には `retry ... until ... max ... delay ...;` を使えます。複雑な分岐、複数評価、承認を含むループはprocessとFSMで明示することが望ましいです。

## 25. Human-in-the-loop

人間も `device operator : human;` として扱います。承認要求と応答は、内容、実行ID、時刻、承認者を監査できるイベントでなければなりません。

## 26. 承認ポリシー

```vhdl
policy external_write is
    require human_approval
        when action.class = external_write;
end policy;
```

push、メッセージ送信、DB更新、デプロイ、破壊操作などは典型的な外部書き込みです。承認対象の内容が変化した場合、古い承認を再利用してはなりません。

## 27. Location

組み込み配置は `local`、`sandbox`、`private_cloud`、`cloud`、`external` です。locationは情報フロー、ネットワーク、実行方式のポリシー判定に使用します。

## 28. ネットワークポリシー

既定拒否を推奨します。宛先はホスト名等で明示し、デバイス単位で許可します。DNS解決後やリダイレクト後の宛先も実行時に検査しなければなりません。

## 29. ファイルシステムポリシー

デバイスごとに読み取り／書き込み可能な範囲を宣言します。ワークスペース外、資格情報ディレクトリ、システム領域は既定で拒否し、シンボリックリンク等による境界越えも防止しなければなりません。

## 30. クラウド送信

`export value to cloud_device;` は監査対象のセキュリティ重要操作です。通常のsignal接続が許される実装でも、クラウド向けデータは単一の外向き通信境界を通し、実ペイロードを再検査しなければなりません。

## 31. Assertion

`assert never`、`assert always` 等により、機密データのクラウド流出、未承認外部書き込み、反復上限超過などを禁止します。静的に証明できないassertionは実行時に監視します。

## 32. 時相Assertion

```vhdl
assert eventually (task.started -> task.completed)
within 30 min;
```

将来版ではLTL/PSL互換の部分集合を検討します。

## 33. Testbench

testbenchは入力、時間進行、期待状態を記述し、実デバイスの代替物とともに設計をシミュレーションします。外部副作用は既定で模擬しなければなりません。

## 34. 障害注入

`inject device.timeout probability 0.05;` のように、タイムアウト、レート制限、ツール失敗を注入し、失敗経路と停止性を検査します。再現のため乱数seedを記録することが望ましいです。

## 35. セキュリティシミュレーション

悪意あるプロンプト、ツール出力、情報抽出要求を注入し、機密データが許可外へ流れないことをassertionで検査します。シミュレーション合格は実行時検査を不要にしません。

## 36. MCPデバイス定義

```vhdl
device tool : mcp is
    generic (
        transport => stdio,
        command   => "example-mcp"
    );
    capability ("evaluate", "read_result");
    clearance restricted;
end device;
```

初期transportは `stdio` と `HTTP` です。サーバー全体ではなくツール単位の能力を宣言しなければなりません。

## 37. ツール単位のCapabilityポリシー

能力にはファイルシステム、ネットワーク、作用クラスを関連付けます。推奨作用クラスは `safe`、`read`、`write`、`external_write`、`destructive` です。未宣言能力は使用不可とします。

## 38. Agent

agentはモデル設定と、明示的に付与されたツール集合を持つdeviceです。agentは列挙されていないツールや権限を取得してはならず、その出力を制御命令として無条件に信頼してはなりません。

## 39. 決定的Judge

完了判定には、ビルド成否、失敗テスト数、lintエラー数など、可能な限り決定的な検査を使います。AIによる評価を使う場合も、それだけを不可逆な作用の根拠にしないことが望ましいです。

## 40. 完了条件

`completion when ...;` で合格を証明する条件を宣言します。モデルが生成した `COMPLETE` 等の文字列だけを完了証明としてはなりません。

## 41. Configuration

configurationはワークフロー論理を変更せずに、抽象deviceをローカル実装、クラウド実装、模擬実装等へ再束縛します。再束縛後も型、機密区分、能力、ポリシーに適合しなければなりません。

## 42. 混成ワークフロー例

典型的なhybrid設計は、restricted情報を扱えるローカルcoder、sandbox内のcompiler/tester、許可済みの機密解除器、internal情報だけを受け取れるcloud reviewerを接続します。候補をビルド・テスト・無害化し、安全な差分だけをクラウドへ送り、全結果をjudgeで評価します。失敗時は診断をcoderへ戻しますが、反復数を制限します。元ソースからcloud reviewerへの直接フローは禁止assertionで表します。

## 43. コンパイラパイプライン

```text
AWHDLソース -> Lexer/Parser -> AST
  -> 型・port・情報フロー・capability・cycle・budget・assertion検査
  -> バージョン付きAWHDL IR
  -> simulator / local runtime / distributed backend
```

未対応構文や意味を黙って落としてはなりません。

## 44. 中間表現

IRは機械可読なJSON等で表現できますが、人間に直接記述を要求しません。IRはschema version、ソース位置、device、signal、process、policy、予算、assertionを保持しなければなりません。MCP通信はJSON/JSON-RPCのままとします。

## 45. ランタイムイベントモデル

イベントキューからschedulerが論理的に並行なprocess、timer、deviceを起動します。各イベントは設計実行、device呼び出し、因果関係を識別できるIDを持つことが望ましいです。

## 46. デルタサイクル意味論

1サイクルを「イベント確定、process評価、signal更新の確定、新イベント検出」とし、必要なら次のデルタサイクルへ進みます。同一デルタ内でのsignal更新はまとめて反映し、ソース記述順に依存しない決定的動作を目指します。

## 47. 永続状態

`signal context : context_type persistent;` はチェックポイント対象です。通常signalは揮発性です。復旧時はIR版、設定、入力来歴との整合を検査しなければなりません。

## 48. 設計原則

1. LLMはdeviceであり、信頼済み制御プレーンではない。
2. セキュリティはプロンプトでなく、型、ポリシー、サンドボックス、ランタイム制御で強制する。
3. 時間を第一級概念とする。
4. agentとMCPは非同期・並行に動作すると仮定する。
5. 外部副作用を明示する。
6. 人間の承認を第一級イベントとする。
7. ループを予算で制限する。
8. JSONはprotocolとIRに使い、人向け主要言語にはしない。
9. 実行前にシミュレーション可能にする。
10. 記述を実行バックエンドから独立させる。
11. ループを観測・比較・改善できる工学的成果物として扱う。

## 49. 最小実用言語 v0.1

最初の対象はentity、architecture、device、signal、process、if、case、timer、timeout、parallel、機密区分、location、budget、assertです。区分は `public < internal < restricted`、配置はlocal/cloud、transportはstdio/HTTPから開始します。

## 50. 推奨実装

初期コンパイラ／ランタイムにはRustを推奨します。PestまたはChumskyで解析し、Rust AST、型・セキュリティ検査、IRを経てTokioイベントランタイムへ接続します。process、signal、timer、parallel、eventをasync taskへ直接同一視せず、AWHDLの決定的意味論をschedulerが仲介します。

## 51. ロードマップ

### v0.1

parser、AST、主要宣言、timer/timeout、基本機密ラベル、local/cloud、MCP stdio、event runtime、基本assertion、基本budget。

### v0.2

人間承認、declassifier、persistent signal、sandbox/network/filesystem policy。

### v0.3

testbench、障害注入、セキュリティシミュレーション、時相assertion。

### v1.0

分散実行、形式検証、ワークフロー合成、複数バックエンド、グラフィカルな回路／ワークフロー表現。

## 52. 長期的位置付け

AWHDLは主としてプロンプト言語ではなく、正式名称どおりの**Agentic Workflow Harness Description Language**であり、分類上はAgentic System Description Languageです。存在する部品、接続、起動イベント、フィードバック、時間制約、情報の到達可能範囲、承認対象、完了・安全の証明条件を記述します。将来のツールチェーンは同一ソースからsimulatorとcompiler/policy checkerを駆動し、local LLM、MCP、cloud agent、人間をruntimeが統制する構成を目指します。

## 53. AI Conductorとの統合

AI ConductorはAWHDL、compiler/checker、runtime、policy、sandbox、gateway、cvimから構成されます。cvimは薄いUIであり、信頼を要するポリシー判断はAI Conductor本体に置きます。

## 54. ソースとCLI規約

推奨拡張子は `.awhdl` です。暫定CLIは `aic check`、`aic compile`、`aic sim`、`aic run` とします。実装済みコマンドは[実装状況](IMPLEMENTATION_STATUS_ja.md)に従います。

## 55. 時間意味論の明確化

AWHDLはイベント駆動、timer駆動、timeout/deadline制約を区別します。timerはオーケストレーション時間であり、ハードウェア周波数ではありません。通常のフィードバックループは完了イベントを契機に進めます。

## 56. 型としてのセキュリティラベル

signalは概念的に `BaseType<Classification>` です。通常の変換は最強の入力区分を保守的に保持し、明示的に信頼されたdeclassifierだけが区分を下げられます。静的検査後も、クラウド外向き通信の実行時検査は必須です。

## 57. 合成

AWHDLでの合成は次を意味します。

```text
AWHDL -> AST -> 静的検査 -> AI Conductor IR -> 実行計画 -> runtime
```

ハードウェア合成を意味しません。
