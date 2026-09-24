# Phase 4 migration and verification

[日本語（正本）](MIGRATION_PHASE_4_ja.md) | English reference translation

> The Japanese version is canonical. This English version is a reference
> translation; where they differ, the Japanese version prevails.

> Superseded for the checkpoint format by [Phase 5](MIGRATION_PHASE_5.md) (v5).

Phase 4 binds human approval to one exact Effect instance. The authoritative
checkpoint is now [`execution-v4.schema.json`](schema/execution-v4.schema.json).
Formats v1–v3 remain historical and `ExecutionState::restore` rejects them.
A v3 checkpoint cannot be upgraded by inventing approval records: it cannot
show which dispatched external write a human authorized. Keep old runs for
inspection and start a new run explicitly.

## Behavior changes

- Effects record `requires_approval`. `EffectRequest::new` defaults it to true
  for `external_write` and `destructive`. `with_approval(false)` opts out,
  except for `destructive`.
- `start_effect` consumes a granted, unexpired approval whose hash matches the
  recomputed canonical action, in the same transition as `STARTED`. Without one,
  the transition fails and nothing is persisted: no invocation, no STARTED, no
  provider call.
- `RunStore` adds `reserve_effect`, `request_approval` and `decide_approval`.
  Callers reserve the Effect, request approval with the exact parameters, obtain
  a trusted decision, then call `invoke_effect`.
- Route configuration adds `human_approval`. `open_data_acquisition` (Codex,
  `external_write`) is explicitly `not_required`, which keeps its pre-Phase-4
  behavior until an approval adapter exists. MATLAB and KDB are `local_write`
  and are not affected. Any new `external_write` MCP route without the setting
  will fail closed.

## Limits

- No approval adapter is connected to the engine. HumanPort is not in the
  runtime MCP inventory, and its `human.answer` tool must be separated from the
  agent-visible surface before it can be trusted as an approver.
- `transaction` and `time_limited_session` scopes are not implemented.
- `capability` is `<class>:<resource>` until Phase 5 defines parameterized
  capabilities. Approval does not bypass later capability checks.
- Canonical encoding is serde_json with sorted keys, not RFC 8785.
- Approval expiry uses the orchestrator's wall clock.

## Verification

From `awhdl/`: `cargo test --workspace --offline`, `cargo fmt --check`, and
`cargo clippy --workspace --all-targets --offline -- -D warnings`. The offline
`dataflow_checkpoint` example emits a v4 checkpoint with a consumed and a
pending approval for schema validation. No live provider or human is required.

> **Update (2026-09-23):** The HumanPort approver adapter is connected and the
> `open_data_acquisition` opt-out has been removed. Each open-data acquisition
> now waits for a human decision in the HumanPort window.
