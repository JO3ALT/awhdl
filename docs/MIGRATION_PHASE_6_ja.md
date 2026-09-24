# Phase 6 の移行と検証

日本語（正本） | [English reference translation](MIGRATION_PHASE_6.md)

> 日本語版を正本とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。

Phase 6 では、決定的な（hard）完了条件と、モデルが生成する（soft）信号を分離した。checkpoint の形式は変わらない（v5）。
完了のポリシーは設定にあり、評価はそのたびに監査に記録する。

## 振る舞いの変更

- 主制御の `complete` だけでは実行は終わらない。runtime が完了のポリシーを評価し、hard の条件が満たされていなければ
  `COMPLETION_REJECTED` を主制御に返し、反復の予算の範囲でループを続ける。
- すべての実行は暗黙に `effects_resolved` を要求する。`UNCERTAIN` か `STARTED` の Effect がある実行は、完了ではなく
  `BLOCKED` で終わる。
- `open_data_population` は safety-critical で、3 つの手順の成功を要求する。`cvim_full_loop` は 6 つの MCP の手順を要求する。
  そのモデルの手順（`initial_coding`）は soft で、失敗すると `REQUIRES_REVIEW` になる。
- workflow のない自由な実行は、最上位の `[completion]` ポリシーを使う。既定では空で、完了に必要なのは Effect が解決して
  いることだけである。調査や質問応答の実行では、これまでの振る舞いを保つ。これらは完了を通じて外部への書き込みを起こせない。
- 設定の読み込みは、モデル由来の hard 条件、条件中の未知の action、external / destructive の手順を含むのに
  safety-critical でない workflow を拒否する。
- 主制御への指示には、完了が検査され、拒否されうることを明記している。

## 制限

- 最終回答はまだ LLM の文章で、hard 条件はそれを検証しない。
- `structured:` の条件は、現在のプロセスにある action ごとの最新の構造化出力を読む。checkpoint からは復元しない（CLI での再開はない）。
- 条件は設定の構文である。AWHDL の `completion when hard {...} soft {...}` のソース構文は、まだ解析しない
  （手順書が認めるとおり、意味論を先に確定した）。

## 検証

`awhdl/` で：`cargo test --workspace --offline`、`cargo fmt --check`、
`cargo clippy --workspace --all-targets --offline -- -D warnings`。スキーマの変更も実際の provider も不要である。
