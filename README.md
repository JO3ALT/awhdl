# AWHDL

## Agentic Workflow Harness Description Language

日本語（正本） | [English reference translation](README_en.md)

**AWHDLはAgentic Workflow Harness Description Language（エージェント型ワークフロー・ハーネス記述言語）の略称です。** 複数のAI、エージェント、MCPツール、決定的プログラム、そして人間を組み合わせた**反復ループを設計するための言語**です。VHDLの考え方を参考に、AIワークフローを型付き・イベント駆動のシステムとして記述します。

本リポジトリには、ドラフトv0.2仕様とRust製の実装が含まれます。`aic check` が v0.1 Profile の構文を解析・静的検査し、`aiconductor run-design` が設計を AI Conductor の route に束縛して delta cycle で実行します。入門には [AWHDL チュートリアル（PDF）](docs/tutorial/awhdl_tutorial_ja.pdf) を参照してください。

## AWHDLが目指すもの

AWHDLの目的は、単一のAIに仕事を依頼することではありません。特性の異なる複数のAIとMCPサーバーを接続し、検証・修正・再実行を含むループ全体を、明示的で検査可能な設計として表すことです。

その根底にある目標は、**ループエンジニアリングに資すること**です。ここでいうループエンジニアリングとは、AIとツールが繰り返し相互作用する系について、接続構造、状態遷移、評価基準、停止条件、資源上限、安全境界を設計し、実行前に解析し、シミュレーションと観測によって検証し、得られた証拠を基に改善する工学的方法を指します。偶然うまくいくプロンプト列ではなく、再現可能な設計対象としてループを扱えるようにすることがAWHDLの中心的な狙いです。

例えば、次のようなループを記述対象とします。

```text
課題
  -> コーディングAI
  -> コンパイラ、テスト、証明器（MCP経由で並列実行）
  -> レビューAI
  -> 決定的な合否判定
  -> 診断を付けて再試行、人間の承認要求、または完了
```

各AIやMCPは、AWHDL上では入出力と能力を持つ `device` として扱います。`signal` がデータや状態を運び、`process` がイベントに反応し、状態機械やフィードバック構造が次の処理を決めます。これにより、プロンプトの連結だけでは曖昧になりやすい次の事項を設計として固定できます。

- どのAIまたはツールが、どのデータを受け取れるか
- ローカルAI、クラウドAI、決定的ツール、人間をどこに配置するか
- 何を契機に処理を開始し、どの処理を並列化するか
- テスト結果やレビュー結果を、どの条件で次の反復へ戻すか
- 再試行回数、時間、トークン、費用などの予算をどう制限するか
- どの時点を完了とし、誰または何が合否を判定するか
- 機密データをどこまで流してよいか、外部送信に承認が必要か
- 実行根拠、来歴、判断履歴をどのように監査可能に残すか

AWHDLはプロンプト記述言語でも、単なる逐次実行スクリプトでもありません。AIを信頼済みの制御主体にせず、AI Conductorランタイムがスケジューリング、情報フロー、能力、予算、期限、承認、完了条件を強制します。AIは結果や提案を生成しますが、実行を継続してよいか、外部へデータを送ってよいか、完了と見なせるかは、宣言された規則と信頼されたランタイムが決定します。

この分離により、長時間動作する複数AIループを、安全性、再現性、試験可能性、監査可能性、モデルやツールの交換可能性を備えたものにすることを目指します。対象例には、コンパイル・テスト・レビューを反復するコード生成、複数経路で検索と検証を行う調査、形式証明、シミュレーションや工学ツールを含む処理、ローカル処理を基本として必要な箇所だけクラウドAIを使う処理があります。

AWHDLはハードウェア記述言語の並行性・信号・イベントという考え方に着想を得ていますが、電子回路を記述する言語ではなく、ハードウェア合成を目的としません。

名称中の **Harness** は、複数の実行主体を接続するだけでなく、そのデータフロー、起動条件、反復、予算、権限、安全境界を一つの制御構造として束ねるもの、という意味です。したがってAWHDLの「H」はHardwareではありません。ハードウェア記述言語から有用な設計概念を借りながら、AI・MCPループのためのハーネスを記述することを表しています。

## 名称

- **AI Conductor**: オーケストレーションシステム全体
- **AWHDL**: Agentic Workflow Harness Description Language。AI・MCPループのハーネス記述言語
- **cvim**: Vim向けの薄いフロントエンド
- **aic**: 暫定的なコマンドライン実行ファイル名

## クイックスタート

必要なものはRust 2024 edition対応ツールチェーンとCargoです。Milestone 1には、特定のモデルサーバー、API資格情報、独自ツール、GPU、マシン固有サービスは不要です。

```bash
cargo test --workspace
cargo run -p conductor-cli -- check examples/hello.awhdl
cargo run -p conductor-cli -- check examples/hello.awhdl --output json
cargo run -p conductor-cli -- graph examples/tutorial/05_parallel_barrier.awhdl --view behavior
cargo run -p conductor-cli -- graph examples/secure_cooperation.awhdl --view petri --format dot | dot -Tsvg -o petri.svg
```

解析または構造検査に失敗すると、ソース位置を示して0以外の終了状態を返します。`aic graph` は検査を通った設計を Mermaid・Graphviz DOT・JSON の図にします（structure・behavior・petri・security・activity の各ビュー。activity は PlantUML、petri は PNML でも出力。[図示](docs/GRAPH_ja.md)）。`examples/secure_cooperation.awhdl` は、ローカル LLM で匿名化したデータを、決定的な検査と人の承認を経て機密解除し、クラウドの AI が解析する公式サンプルです。

## ドキュメント

| 文書 | 日本語 | English |
| --- | --- | --- |
| AWHDL言語仕様 | [日本語](docs/AWHDL_LANGUAGE_SPEC_v0.2_ja.md) | [English](docs/AWHDL_LANGUAGE_SPEC_v0.2.md) |
| AI Conductorシステム仕様 | [日本語](docs/AI_CONDUCTOR_SYSTEM_SPEC_v0.2_ja.md) | [English](docs/AI_CONDUCTOR_SYSTEM_SPEC_v0.2.md) |
| はじめに | [日本語](docs/GETTING_STARTED_ja.md) | [English](docs/GETTING_STARTED.md) |
| チュートリアル（TeX / PDF、実機で確認した例つき） | [PDF](docs/tutorial/awhdl_tutorial_ja.pdf)・[TeX](docs/tutorial/awhdl_tutorial_ja.tex) | — |
| 実装状況 | [日本語](docs/IMPLEMENTATION_STATUS_ja.md) | [English](docs/IMPLEMENTATION_STATUS.md) |
| 図示（`aic graph`） | [日本語](docs/GRAPH_ja.md) | [English](docs/GRAPH.md) |
| セキュリティモデル | [日本語](docs/SECURITY_MODEL_ja.md) | [English](docs/SECURITY_MODEL.md) |
| 設計判断 | [日本語](docs/DESIGN_DECISIONS_ja.md) | [English](docs/DESIGN_DECISIONS.md) |
| 公開前チェックリスト | [日本語](docs/PUBLICATION_CHECKLIST_ja.md) | [English](docs/PUBLICATION_CHECKLIST.md) |
| cvimの責務境界 | [日本語](cvim/README_ja.md) | [English](cvim/README.md) |
| 実装ガイド | [日本語](IMPLEMENTATION_GUIDE_v0.2_ja.md) | [English](IMPLEMENTATION_GUIDE_v0.2.md) |
| v0.1 Profile（範囲と状態の正本） | [日本語](docs/PROFILE_v0.1_ja.md) | [English](docs/PROFILE_v0.1.md) |
| 言語仕様（規範の入口） | [日本語](docs/LANGUAGE_SPEC_ja.md) | [English](docs/LANGUAGE_SPEC.md) |
| Runtime 仕様 | [日本語](docs/RUNTIME_SPEC_ja.md) | [English](docs/RUNTIME_SPEC.md) |
| Security 仕様 | [日本語](docs/SECURITY_SPEC_ja.md) | [English](docs/SECURITY_SPEC.md) |
| IR 仕様 | [日本語](docs/IR_SPEC_ja.md) | [English](docs/IR_SPEC.md) |
| 実装ガイド（v0.1） | [日本語](docs/IMPLEMENTATION_GUIDE_ja.md) | [English](docs/IMPLEMENTATION_GUIDE.md) |
| 仕様整理の設計判断 | [日本語](docs/REFINEMENT_DECISIONS_ja.md) | — |

日本語版を規範文書（正本）とし、英語版は理解と国際的な共有を助けるための参考訳とします。両者に差異がある場合は日本語版が優先されます。v0.1 の実装範囲と各機能の状態は [PROFILE_v0.1](docs/PROFILE_v0.1_ja.md) の表だけが定めます（テストでコードとの対応を検査）。[実装状況](docs/IMPLEMENTATION_STATUS_ja.md)はその要約です。

## ワークスペース

- `awhdl-ast`: AST定義とソース上のバイト範囲
- `awhdl-parser`: Pest文法とAST構築
- `awhdl-checker`: 構造・情報フロー・Profile の静的検査
- `awhdl-graph`: 検査済み設計の図示（ビューの射影と Mermaid・DOT・JSON 出力）
- `conductor-cli`: `aic`コマンドラインフロントエンド
- `aiconductor-runtime`: AWHDL 設計のコンパイルと delta cycle 実行、Effect・承認・capability・完了判定を持つ runtime（`aiconductor run-design`）

## 開発時の検査

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## 現在の制限

未達の項目は [PROFILE_v0.1](docs/PROFILE_v0.1_ja.md) の「v0.1 の未達と既知の逸脱」を参照してください。runtime のテストは同梱の匿名化した設定（`crates/aiconductor-runtime/tests/fixtures/project`）で動き、外部の LLM・MCP・人間は不要です。実機の device を使う確認は `examples/` の live 例で行います。未対応構文は診断を出して拒否し、黙って受理しません。

## ライセンス

AWHDLは[MIT License](LICENSE)で提供します。
