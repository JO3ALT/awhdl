# AWHDLにおけるcvimの責務境界

[English](README.md) | 日本語

> この日本語版を規範文書（正本）とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。

cvimはVim向けの薄いフロントエンドです。`.awhdl`ファイルを`aic check`へ渡して診断を表示でき、将来はcompile、sim、run操作も起動できます。

cvimはAWHDLの解析、信頼を要するスケジューリング判断、プロジェクトへの権限付与、クラウド外向き通信ポリシーの迂回、サンドボックス強制を行ってはいけません。これらはAI Conductorの責務です。

Milestone 1での使用方法:

```bash
aic check path/to/workflow.awhdl
aic check path/to/workflow.awhdl --output json
```
