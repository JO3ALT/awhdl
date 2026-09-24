# Phase 1 の移行と検証

日本語（正本） | [English reference translation](MIGRATION_PHASE_1.md)

> 日本語版を正本とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。

Phase 1 の記録（履歴）です。checkpoint の形式は Phase 2 で置き換わり、payload を持つデータフローの保存が加わりました。
[Phase 2 の移行](MIGRATION_PHASE_2_ja.md) を参照してください。

- root の v0.1 仕様と、言語・システムの v0.2 の 8 つの写しすべてから、実行 identity に関する規範的な追補へ
  リンクしている。再開時のファイル読み込みは成功した。
- 既存の AWHDL ソースと parser / AST は変更していない。新しい構文を黙って受理することはない。
  構造検査器は構造の検査だけを行う。
- 新しい実行は correlation と invocation のメタデータを割り当てる。監査には、identity と結果を持つ
  `invocation_started` / `invocation_finished` の記録が加わる。
- `state.json` は `execution` を必須とする。新しい `execution.json` は、dispatch と完了のたびに、
  正となる identity の状態を保存する。identity を持たない古い checkpoint を、ID をでっち上げて再開してはならない。
  監査のために保存し、新しい実行は呼び出し側の明示的な判断でのみ始める。
- `ExecutionState::restore` は、呼び出しを発行せずに identity を復元する。CLI での再開、AWHDL のコンパイルと実行、
  完全な signal の保存、並列スケジューラは実装していない。
- 回帰テストは、遅れて届いた完了、再試行、兄弟のスコープ、世代が混在する barrier の拒否、偽造・再送された identity、
  checkpoint の復元、dispatch 前の永続化、監査メタデータ、エラーと再試行の identity を対象とする。
- 必要な検証（`awhdl/` で実行）：`cargo test --workspace`、`cargo fmt --check`、
  `cargo clippy --workspace --all-targets -- -D warnings`。

手順は 1 Phase ずつ適用している。Phase 2〜7 は未着手。

2026-09-23 の再開後の検証：workspace の全 33 テスト（runtime のテスト 29 件を含む）が通り、workspace の書式検査と、
警告を拒否する workspace の Clippy も通った。テストと Clippy は `--offline` で実行した。以前に起きた fixture の読み込み失敗は
再発しなかったが、原因は確認できていない。上に記した実装の範囲内で、Phase 1 の検証は完了した。進捗と再開の手順も参照のこと。
