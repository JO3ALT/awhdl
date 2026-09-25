# aic graph の検証ツール（開発用）

[English](README.md) | 日本語

> この日本語版を正本とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。

`aic graph` の出力を、実際の描画ツールで検査するための開発用ツールです。すべての例を、すべてのビューと形式で
出力し、それぞれが正しく解析できるかを確かめます。`--render` を付けると PNG にも描画します。

| 形式 | 検査 | 描画（`--render`） |
|---|---|---|
| Mermaid | Mermaid ライブラリの `mermaid.parse` | ヘッドレス Chrome（Puppeteer）上の Mermaid |
| DOT | pydot | Graphviz の WASM 版（@hpcc-js/wasm-graphviz） |
| PlantUML | `plantuml -checkonly` | PlantUML |
| PNML | pm4py の `read_pnml`（P/T ネットとして読み込む） | — |
| JSON | `python3 -m json.tool` | — |

## 使い方

```bash
sh tools/graph-validate/setup.sh             # 初回だけ。Node 18+、Python 3.10+、Java 11+、curl が必要
sh tools/graph-validate/validate.sh          # 検査
sh tools/graph-validate/validate.sh --render # 検査と PNG の描画
AIC=target/debug/aic sh tools/graph-validate/validate.sh   # ビルド済みの aic を使う
```

ツールと出力は、既定でリポジトリの外（`~/.cache/awhdl/graph-validate`、`AWHDL_GRAPH_TOOLS` で変更可）に置きます。
OneDrive などの同期フォルダに `node_modules` を作らないためです。

## 固定したバージョンとライセンス

バージョンは `package.json`、`requirements.txt`、`fetch-plantuml.sh`（SHA-256 で検証）で固定しています。
これらはリポジトリに同梱せず、`setup.sh` がインストールします。いずれも開発時の検査だけに使い、`aic` や
runtime の依存ではありません。

| ツール | バージョン | ライセンス |
|---|---|---|
| mermaid | 12.0.0 | MIT |
| puppeteer（ヘッドレス Chrome を取得） | 22.15.0 | Apache-2.0（Chrome は Chrome の利用条件） |
| @hpcc-js/wasm-graphviz | 1.29.1 | Apache-2.0（Graphviz は EPL） |
| jsdom / dompurify | 22.1.0 / 3.4.16 | MIT / Apache-2.0 または MPL-2.0 |
| pydot | 4.0.1 | MIT |
| pm4py | 2.7.23.8 | AGPL-3.0 |
| PlantUML | 1.2025.4 | GPL-3.0 |

pm4py（AGPL）と PlantUML（GPL）は、手元で検査に使うだけで、配布物に含めません。
