# AWHDL / AI Conductor Profile v0.1

日本語（正本） | [English reference translation](PROFILE_v0.1.md)

> **正本の範囲。** 規範文書: [LANGUAGE_SPEC](LANGUAGE_SPEC_ja.md)、
> [RUNTIME_SPEC](RUNTIME_SPEC_ja.md)、[SECURITY_SPEC](SECURITY_SPEC_ja.md)、
> [IR_SPEC](IR_SPEC_ja.md) と本 Profile。非規範: [IMPLEMENTATION_GUIDE](IMPLEMENTATION_GUIDE_ja.md)、
> 例、チュートリアル、移行メモ、状態報告。矛盾時は本 Profile、次に各規範文書が優先する。
> 日本語版を正本とし、英語版は参考訳とする。差異がある場合は日本語版が優先する。

本書は、v0.1 が実装しなければならない範囲を定める**唯一**の文書である。他の文書は v0.1 の範囲を
追加も削除もしてはならない。ここに記載のない機能、または Profile が `v0.2` の機能は v0.1 の範囲外であり、
対応する [設計判断](REFINEMENT_DECISIONS_ja.md) を記録したうえで本書に移すまで実装してはならない。

## 範囲の規則

基本範囲は仕様整理手順書が推奨する v0.1 の subset に従う。本システムはすでに外部書き込み
（`open_data_acquisition`、full access の Codex）を許可しているため、手順書の規則どおり、Effect journal・
idempotency・照合（reconciliation）・永続 checkpoint・action に bind した承認を先送りせず v0.1 に**含める**。
基本的な capability 検査には、planner が渡すファイルパスの範囲制限を含める。これがなければ capability
検査が意味をなさないためである。

v0.1 の行は、該当するすべての列が `yes` のときに限り**適合**とする。すべての v0.1 行が適合したとき
v0.1 は完了する。現時点では完了しておらず、残りは下表が記録する。

## 機能の状態表

列: **Parse** と **Static** は AWHDL ソース（parser、checker）、**Runtime** は Rust runtime、**Tested** は
自動テストが実行することを表す。値は `yes`、`no`、`partial`、`n/a`。**Evidence** はテスト関数名であり、
Profile テストがその実在と、`yes` / `partial` の実装主張がテストに裏付けられていることを検査する。
表は日英で同一に保つ（機能名・値・根拠は識別子として英語のまま記す）。

| Feature | Area | Profile | Parse | Static | Runtime | Tested | Evidence |
|---|---|---|---|---|---|---|---|
| entity | language | v0.1 | yes | yes | yes | yes | `parses_minimal_hello_design`, `accepts_the_minimal_hello_structure`, `pipeline_runs_processes_in_delta_cycles` |
| architecture | language | v0.1 | yes | yes | yes | yes | `parses_minimal_hello_design`, `accepts_the_minimal_hello_structure`, `pipeline_runs_processes_in_delta_cycles` |
| device declaration | language | v0.1 | yes | yes | yes | yes | `rejects_an_unknown_device_call`, `parses_v01_declarations_statements_and_expressions`, `binding_checks_kind_location_route_and_budget_support` |
| process with sensitivity list | language | v0.1 | yes | partial | yes | yes | `selected_sensitivity_remains_an_untyped_name_and_event_declarations_are_unsupported`, `pipeline_runs_processes_in_delta_cycles` |
| signal / Value | language | v0.1 | yes | yes | yes | yes | `parses_v01_declarations_statements_and_expressions`, `values_change_only_on_assignment_of_a_different_value`, `conflicting_drivers_in_one_delta_are_rejected` |
| Event | language | v0.1 | no | no | yes | yes | `equal_payload_events_remain_distinct_and_consumption_survives_replay`, `timeouts_fail_calls_and_run_on_timeout_handlers` |
| Invocation (device call) | language | v0.1 | yes | yes | yes | yes | `rejects_an_unknown_device_call`, `device_identity_is_durable_before_dispatch_and_completion_is_bound` |
| if | language | v0.1 | yes | yes | yes | yes | `parses_v01_declarations_statements_and_expressions`, `feedback_loop_stops_by_condition_and_assertions_hold` |
| timer | language | v0.1 | yes | yes | yes | yes | `parses_v01_declarations_statements_and_expressions`, `timers_need_budgets_and_budgets_end_the_run` |
| timeout | language | v0.1 | yes | yes | yes | yes | `parses_v01_declarations_statements_and_expressions`, `timeouts_fail_calls_and_run_on_timeout_handlers` |
| parallel / barrier | language | v0.1 | yes | yes | partial | yes | `parallel_results_commit_together_and_barrier_fires`, `structural_rules_for_v01_constructs`, `parallel_barrier_requires_matching_scope_and_all_branches` |
| budget | language | v0.1 | yes | yes | yes | yes | `parses_v01_declarations_statements_and_expressions`, `timers_need_budgets_and_budgets_end_the_run`, `rejects_iteration_overrun` |
| basic assert | language | v0.1 | yes | yes | yes | yes | `parses_v01_declarations_statements_and_expressions`, `feedback_loop_stops_by_condition_and_assertions_hold` |
| hard / soft completion | language | v0.1 | no | no | yes | yes | `hard_failure_blocks_completion_even_when_soft_and_planner_pass`, `completion_policies_are_statically_checked`, `planner_cannot_assert_completion_facts` |
| classification public / internal / restricted | security | v0.1 | yes | yes | yes | yes | `restricted_data_cannot_reach_a_cloud_device`, `unlabeled_data_is_treated_as_restricted`, `features_outside_the_v01_profile_are_reported` |
| location local / cloud | security | v0.1 | yes | yes | yes | yes | `features_outside_the_v01_profile_are_reported`, `binding_checks_kind_location_route_and_budget_support` |
| static restricted to cloud deny | security | v0.1 | n/a | yes | yes | yes | `restricted_data_cannot_reach_a_cloud_device`, `declared_never_flow_lowers_the_cloud_floor`, `cloud_egress_is_checked_statically_and_again_at_runtime` |
| deterministic device | device | v0.1 | yes | partial | yes | yes | `binding_checks_kind_location_route_and_budget_support`, `scopes_a_deterministic_action_to_its_markdown_section` |
| agent device (LLM, Codex) | device | v0.1 | yes | partial | yes | partial | `pipeline_runs_processes_in_delta_cycles`, `codex_adapter_enforces_route_security_settings`, `parses_plain_and_fenced_decisions` |
| MCP device over stdio | device | v0.1 | yes | partial | yes | yes | `normalizes_and_limits_mcp_output`, `humanport_approver_mode_hides_answering_and_waits`, `matlab_file_is_sent_as_code_after_the_capability_check` |
| run / invocation / correlation identity | runtime | v0.1 | n/a | n/a | yes | yes | `retry_has_new_identity_and_incremented_attempt`, `foreign_forged_duplicate_and_superseded_results_are_rejected` |
| generation | runtime | v0.1 | n/a | n/a | yes | yes | `late_results_cannot_enter_a_new_generation`, `new_generation_cannot_read_old_values_or_deliver_old_events` |
| basic event queue | runtime | v0.1 | n/a | n/a | yes | yes | `pipeline_runs_processes_in_delta_cycles`, `event_is_durably_consumed_before_delivery_and_values_survive` |
| AWHDL to runtime compilation | runtime | v0.1 | n/a | n/a | yes | yes | `pipeline_runs_processes_in_delta_cycles`, `binding_checks_kind_location_route_and_budget_support` |
| persistent checkpoint | runtime | v0.1 | n/a | n/a | yes | yes | `checkpoint_restores_pending_identity_without_reissuing`, `checkpoint_failure_does_not_deliver_or_allow_further_dispatch` |
| Effect journal and UNCERTAIN | runtime | v0.1 | n/a | n/a | yes | yes | `crash_after_started_requires_reconciliation_before_retry`, `timeout_blocks_redelivery_until_trusted_not_found_reconciliation` |
| idempotency key | runtime | v0.1 | n/a | n/a | yes | yes | `started_is_durable_before_provider_call_and_confirmed_result_is_reused` |
| reconciliation (trusted API) | runtime | v0.1 | n/a | n/a | yes | yes | `reopened_run_persists_uncertain_then_reconciles_without_redelivery`, `provider_error_remains_uncertain_until_confirmed_by_reconciliation` |
| action-bound human approval | security | v0.1 | n/a | n/a | yes | partial | `external_write_needs_bound_approval_and_consumes_it_once`, `approval_bound_writes_ask_the_human_before_dispatch`, `humanport_approver_mode_hides_answering_and_waits`, `task_carries_the_canonical_action_and_answers_are_strict` |
| basic capability check | security | v0.1 | n/a | n/a | yes | yes | `capabilities_unify_tool_file_sandbox_and_model_policy`, `capability_policy_changes_are_validated_against_routes` |
| filesystem scope for planner paths | security | v0.1 | n/a | n/a | yes | yes | `planner_paths_outside_granted_scopes_are_denied`, `workspace_paths_are_scoped_and_outside_paths_are_denied` |
| audit JSONL, metadata only | security | v0.1 | n/a | n/a | yes | yes | `run_store_dispatches_only_with_a_persisted_single_use_approval` |
| declassifier | security | v0.2 | no | no | no | no | — |
| HTTP MCP transport | device | v0.2 | n/a | n/a | no | no | — |
| per-host network policy enforcement | security | v0.2 | n/a | n/a | no | no | Codex network is all-or-nothing (host:* only). |
| cloud DLP / egress inspection | security | v0.2 | n/a | n/a | no | no | — |
| taint / information-flow check | security | v0.2 | no | no | no | no | — |
| advanced cancellation | runtime | v0.2 | no | no | partial | no | Runtime can mark an Invocation cancelled; no provider stop guarantee. |
| CLI resume and run lock | runtime | v0.2 | n/a | n/a | no | no | — |
| transaction / time-limited approval scopes | security | v0.2 | n/a | n/a | no | no | — |
| automatic provider reconciliation lookup | runtime | v0.2 | n/a | n/a | no | no | — |

## v0.1 の未達と既知の逸脱

表から導かれる未達の v0.1 行:

- **Event 宣言と完了条件の構文:** 意味論は確定・実装済みだが、ソース構文が未定（LANGUAGE_SPEC）の
  ため Parse は `no`。
- **partial の静的検査:** `x.changed` などの感度メンバーは頭の名前だけで解決する。device の種類と
  route への束縛は checker ではなくコンパイル時（runtime の束縛）に検査する。
- **parallel:** 結果は delta cycle の終わりにまとめて反映されるが、v0.1 の runtime は呼び出しを
  1つずつ順に行う。
- **agent device と人間の承認のテスト:** 実モデルによる live 実行は未確認。HumanPort の GUI による
  回答経路は手動で1回確認した（`live_approval_smoke`、2026-09-23）。自動テストは adapter、MCP 側、
  および scripted device による engine の流れを対象とし、GUI のクリック操作は含まない。

承認済みの逸脱は残っていない。`open_data_acquisition` の承認除外は 2026-09-23 に所有者が撤廃したため、
外部書き込み・破壊的操作の route はすべて action に bind した人間の承認を必要とする。

## 実装者と agent への規則

1. v0.1 の行だけを実装する。行を v0.1 に移すことは本書の変更と設計判断の記録を伴い、コードより先に行う。
2. 状態を変える変更と同じ変更で該当行を更新する。状態は本書にだけ記録し、他の文書の文章では管理しない。
   状態報告は要約してよいが、本書と食い違ってはならない。
3. Parse・Static・Runtime に `yes` または `partial` と書くには、Evidence にテストが必要である。
   テスト `profile_matrix_is_backed_by_existing_tests` がこれを強制し、日英の表が同一であることも検査する。
4. LANGUAGE_SPEC と IMPLEMENTATION_GUIDE は範囲を定めない。そこに記述があっても、本書にない構成要素や
   `v0.2` の構成要素は v0.1 では未対応である。
