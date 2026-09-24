# Phase 7 の移行

日本語（正本） | [English reference translation](MIGRATION_PHASE_7.md)

> 日本語版を正本とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。

Phase 7 では文書を変更し、整合性のテストを 1 つ加えた。runtime の振る舞い、設定、checkpoint の形式（v5）は変わらない。

## 移動した内容

| 内容 | 以前 | 現在 |
|---|---|---|
| v0.1 の範囲 | `CODEX_IMPLEMENTATION_INSTRUCTIONS.md` のマイルストーン、v0.1 の仕様、実装状況の文章 | `docs/PROFILE_v0.1.md` だけ |
| 機能の状態 | `awhdl/docs/IMPLEMENTATION_STATUS*.md`、進捗メモ | PROFILE_v0.1 の表（状況文書はその要約） |
| 承認、capability | `docs/RUNTIME_SPEC.md` の Phase 4〜5 | `docs/SECURITY_SPEC.md` |
| 監査の規則 | runtime と移行メモに分散 | `docs/SECURITY_SPEC.md` |
| 言語の入口 | root の v0.1 と awhdl の v0.2 の仕様 | `docs/LANGUAGE_SPEC.md` → AWHDL v0.2（日本語が規範） |
| 実装の進め方 | エージェント向けの指示書 | `docs/IMPLEMENTATION_GUIDE.md`（非規範） |
| v0.2 の候補 | — | `docs/PROFILE_v0.2.md`（計画） |

## 新しい義務

機能の状態を変える変更では、同じ変更で PROFILE_v0.1 の該当行を更新する。`profile_matrix_is_backed_by_existing_tests` は、
行がテストなしに `yes` を主張したとき、存在しないテストを名指ししたとき、テストのない `partial` に説明がないときに失敗する。

## 検証

`awhdl/` で：`cargo test --workspace --offline`、`cargo fmt --check`、
`cargo clippy --workspace --all-targets --offline -- -D warnings`。
