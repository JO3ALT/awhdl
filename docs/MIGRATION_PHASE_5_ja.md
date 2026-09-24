# Phase 5 の移行と検証

日本語（正本） | [English reference translation](MIGRATION_PHASE_5.md)

> 日本語版を正本とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。

Phase 5 では、ツール、ファイルシステム、sandbox、ネットワーク、モデルの権限を、1 つのパラメータ付き capability モデルに
統合した。正となる checkpoint は [`execution-v5.schema.json`](schema/execution-v5.schema.json) である。形式 v1〜v4 は
歴史的なものとして残り、復元時に拒否する。既存の Effect に、それを認めた capability を後から割り当てることはできないためである。

## 新しい設定：`config/capabilities.toml`

読み込み時に必須。許可されていないものはすべて拒否する。route の静的な必要が許可されていないとき、許可が未知の route を
名指ししているとき、許可のクラスが route の `effect_class` を超えるとき、設定の読み込みは失敗する。route やツールを
追加するときは、同じ変更で許可を追加すること。

同梱の許可は、以前の振る舞いを再現する。ただし次の変更は意図的なものである。

- **ファイル引数の範囲を限定した。** Lean・Prolog・MATLAB のファイルツールが読めるのは、`examples/**`、`.runtime/runs/**`、
  `.runtime/mcp/**`、`.runtime/open-data/**`、`.runtime/full-loop/**` だけである。以前の runtime は、絶対パスや `..` を
  含め、存在するパスなら何でも受け付けていた。`.runtime/secrets` と `config/` は、これらのツールからは読めない。
- **KDB のクラスはツールごと。** `get_*` と `list_server_limits` は `read` で、Effect journal に入らなくなった。
  `load_*`、`save_*`、`prune_state`、`run_q` は `local_write` のまま。列挙していない KDB のツールは拒否する。
- **Prolog** は `run_prolog*` に、**MATLAB** は自身のサーバーに限る。
- **Codex** の route は明示的な sandbox の capability を持つ。`open_data_acquisition` は `sandbox.full_access` と
  `network.connect host:*` を持つ。Codex は接続先を制限できないため、広い許可として記録している。
- **モデル**は、主制御を含め、route とプロファイルごとに許可する。

`mcp-servers.toml` の `capability` の文字列は説明用で、強制されない。

## 制限

- 機密区分と配置を考慮した認可、クラウドのゲートウェイは実装していない。
- passthrough のツール（KDB、filter）の引数は runtime が検査しない。
- 認証情報の仲介はない。取り消しはポリシーの API と設定の編集であり、動いているエージェントに対する即時の取り消し経路ではない。
- 絞り込みは保守的な包含判定を使うため、正しい部分集合を拒否することがある。

## 検証

`awhdl/` で：`cargo test --workspace --offline`、`cargo fmt --check`、
`cargo clippy --workspace --all-targets --offline -- -D warnings`。オフラインの `dataflow_checkpoint` の例は、ポリシーで認可し、
スキーマ検証用の v5 の checkpoint を出力する。運用の設定は config のテストで検証する。実際の provider の呼び出しは不要である。
