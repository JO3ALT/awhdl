# AWHDL / AI Conductor Profile v0.1

[日本語（正本）](PROFILE_v0.1_ja.md) | English reference translation

> This English document is a reference translation. The Japanese document is
> normative and takes precedence if the versions differ.

> **Source of truth.** Normative: [LANGUAGE_SPEC](LANGUAGE_SPEC.md),
> [RUNTIME_SPEC](RUNTIME_SPEC.md), [SECURITY_SPEC](SECURITY_SPEC.md),
> [IR_SPEC](IR_SPEC.md) and this profile. Non-normative:
> [IMPLEMENTATION_GUIDE](IMPLEMENTATION_GUIDE.md), examples, tutorials,
> migration notes and status reports. On conflict, this profile wins, then the
> normative specs.

This document is the **only** definition of what v0.1 must implement. No other
document may add to or remove from the v0.1 scope. A feature not listed here, or
listed with profile `v0.2`, is out of scope for v0.1 and must not be
implemented until it is moved here with a matching entry in
[DESIGN_DECISIONS](REFINEMENT_DECISIONS_ja.md).

## Scope rule

The base scope follows the refinement procedure's recommended v0.1 subset. The
system already permits external writes (`open_data_acquisition`, Codex with full
access), so, as the procedure requires, the Effect journal, idempotency,
reconciliation, persistent checkpoints and action-bound approval are **in**
v0.1 rather than deferred. Basic capability checks include filesystem scoping of
planner-supplied paths, which is required to make the capability check
meaningful.

A v0.1 row is **conformant** only when every applicable column is `yes`.
v0.1 is complete when all v0.1 rows are conformant. It is not complete today;
the matrix is the record of what remains.

## Feature matrix

Columns: **Parse** and **Static** refer to AWHDL source (parser, checker);
**Runtime** refers to the Rust runtime; **Tested** means an automated test
exercises it. Values: `yes`, `no`, `partial`, `n/a`. **Evidence** names test
functions; the profile test verifies that they exist and that any `yes` or
`partial` implementation claim is backed by tests.

| Feature | Area | Profile | Parse | Static | Runtime | Tested | Evidence |
|---|---|---|---|---|---|---|---|
| entity | language | v0.1 | yes | yes | yes | yes | `parses_minimal_hello_design`, `accepts_the_minimal_hello_structure`, `pipeline_runs_processes_in_delta_cycles` |
| architecture | language | v0.1 | yes | yes | yes | yes | `parses_minimal_hello_design`, `accepts_the_minimal_hello_structure`, `pipeline_runs_processes_in_delta_cycles` |
| device declaration | language | v0.1 | yes | yes | yes | yes | `rejects_an_unknown_device_call`, `parses_v01_declarations_statements_and_expressions`, `binding_checks_kind_location_route_and_budget_support` |
| process with sensitivity list | language | v0.1 | yes | partial | yes | yes | `selected_sensitivity_remains_an_untyped_name_and_event_declarations_are_unsupported`, `sensitivity_names_must_be_events_the_runtime_raises`, `a_call_timeout_wakes_processes_sensitive_to_device_timeout`, `pipeline_runs_processes_in_delta_cycles` |
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
| graph views (aic graph) | tooling | v0.1 | n/a | yes | n/a | yes | `structure_places_devices_in_their_zones`, `security_view_lets_only_cleared_data_reach_the_cloud`, `behavior_view_shows_forks_generation_aware_joins_and_the_retry_loop`, `the_state_view_does_not_invent_states`, `petri_view_is_a_place_transition_net`, `activity_view_has_one_structured_activity_per_process_and_barrier`, `plantuml_and_pnml_render_only_the_view_they_can_express` |
| declassifier | security | v0.2 | yes | yes | yes | yes | `only_a_declassifier_lowers_a_class`, `declassifiers_must_lower_through_trusted_means`, `declassify_releases_only_after_the_filter_and_the_human_allow_it`, `declassification_asks_the_human_approver_with_a_bound_hash` |
| HTTP MCP transport | device | v0.2 | n/a | n/a | no | no | — |
| per-host network policy enforcement | security | v0.2 | n/a | n/a | no | no | Codex network is all-or-nothing (host:* only). |
| cloud DLP / egress inspection | security | v0.2 | n/a | n/a | no | no | — |
| taint / information-flow check | security | v0.2 | n/a | yes | no | yes | `implicit_flows_through_triggers_conditions_and_outcomes_are_rejected`, `restricted_data_cannot_reach_a_cloud_device` |
| advanced cancellation | runtime | v0.2 | no | no | partial | no | Runtime can mark an Invocation cancelled; no provider stop guarantee. |
| CLI resume and run lock | runtime | v0.2 | n/a | n/a | no | no | — |
| transaction / time-limited approval scopes | security | v0.2 | n/a | n/a | no | no | — |
| automatic provider reconciliation lookup | runtime | v0.2 | n/a | n/a | no | no | — |

## v0.1 gaps and known deviations

Open v0.1 rows, derived from the matrix:

- **Event declarations and completion syntax:** semantics are fixed and
  implemented, but source syntax is open (LANGUAGE_SPEC), so Parse is `no`.
- **Static checks marked partial:** device kinds and route binding are checked
  at compilation (runtime binding), not by the checker. Sensitivity names are
  checked to be events the runtime raises (2026-09-25; before that they were
  resolved by root name only).
- **parallel:** results commit together at the end of the delta cycle, but the
  v0.1 runtime dispatches the calls one after another.
- **agent device / human approval tests:** live model runs are not verified;
  the HumanPort GUI answer path was verified manually once
  (`live_approval_smoke`, 2026-09-23). Automated tests cover the adapters, the
  MCP side and the engine flow with scripted devices, not the GUI click.

No approved deviations remain: the `open_data_acquisition` approval opt-out was
removed on 2026-09-23 by the owner, so every external or destructive route
requires action-bound human approval.

## Rules for implementers and agents

1. Implement only v0.1 rows. Moving a row to v0.1 is a change to this document
   plus a DESIGN_DECISIONS entry, made before the code.
2. Update a row in the same change that alters its status. Record status here,
   not in prose elsewhere; status reports may summarize but must not disagree.
3. A `yes` or `partial` in Parse, Static or Runtime requires tests named in
   Evidence. The test `profile_matrix_is_backed_by_existing_tests` enforces this.
4. The LANGUAGE_SPEC and IMPLEMENTATION_GUIDE do not define scope. A construct
   described there but absent or `v0.2` here is unsupported in v0.1.
