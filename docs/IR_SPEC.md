# Execution IR specification

[日本語（正本）](IR_SPEC_ja.md) | English reference translation

> **Source of truth.** Normative: [LANGUAGE_SPEC](LANGUAGE_SPEC.md),
> [RUNTIME_SPEC](RUNTIME_SPEC.md), [SECURITY_SPEC](SECURITY_SPEC.md),
> [IR_SPEC](IR_SPEC.md) and [PROFILE_v0.1](PROFILE_v0.1.md). Non-normative:
> [IMPLEMENTATION_GUIDE](IMPLEMENTATION_GUIDE.md), examples, tutorials,
> migration notes and status reports. On conflict, PROFILE_v0.1 wins, then the
> normative specs. Only PROFILE_v0.1 defines the v0.1 implementation scope.
> This English document is a reference translation; the Japanese document is
> normative and takes precedence if the versions differ.

Scope: versioning, invocation metadata, action descriptors, Value / Event
encoding, Effect and approval state, and the checkpoint format.

Current format: v5 (execution, Effect, approval and capability binding).
Machine-readable schema:
[`schema/execution-v5.schema.json`](schema/execution-v5.schema.json).
The v1–v4 schemas in [`schema/`](schema) are historical; current restore rejects them. This is runtime state, not an AWHDL program IR (see "Compiled design").

`ExecutionState` serializes `version: 5`, `workflow_run_id`, `correlation_id`,
`generation`, `records` (invocation UUID → `Invocation<JSON>`), `dataflow`,
`effects` (Effect UUID → Effect) and `approvals` (approval UUID → approval).
A record has `identity`, `operation`, `status` and `result`. Status is `pending`,
`completed`, `failed` or `cancelled`. Only completed records carry a result,
wrapped as `{"payload": T}` so a successful JSON null survives serialization.
All other results are null. `ResultEnvelope<T>` remains `{identity, payload}`
for returning a device response to its caller; it is not a stored Value or Event.

Identity fields: `workflow_run_id`, `invocation_id`, `parent_invocation_id`,
`causation_id`, `correlation_id`, `generation` (u64), `attempt` (positive u32),
`created_at` (UTC timestamp). Nullable parent references an invocation; nullable
cause references an existing Event in the same scope. IDs are runtime-owned.
Phase 1's identity ownership, retry and generation rules continue to apply.

`dataflow` contains:

- `values`: name → `Value<JSON>` with `scope`, positive `revision`, and `payload`.
  Scope is `{workflow_run_id, correlation_id, generation}`. Latest assignment per
  name is retained; access filters to the current generation. Revision starts at
  1 in each generation and increments only on changed JSON values.
- `events`: append-ordered `Event<JSON>` history. Each has `event_id`, positive
  contiguous `sequence`, `scope`, nullable `causation_id`, `kind`, and `payload`.
  `kind` is a tagged object: `{"kind":"external","topic":"timer"}`,
  `{"kind":"value_changed","name":"candidate","revision":1}`, or
  `{"kind":"invocation_terminated","invocation_id":"<UUID>","status":"completed"}`.
  Terminal statuses also include failed/cancelled. Lifecycle event payload is
  null; the result is in the invocation record. Changed-event payload snapshots
  the assigned value. External payloads may be arbitrary JSON.
- `consumed`: unique event IDs retained as durable consumption markers. Event
  history remains available for validation/causation, never as an unconsumed value.

Restore verifies version, run/correlation scope, invocation identities/parents,
attempt uniqueness, result/status consistency, contiguous event order, unique
IDs, same-scope earlier event causes, known consumed IDs, value revision/content
consistency, and one matching event for every terminal invocation. Older
scopes may be retained in history but cannot be delivered as current events.
Schema validation alone cannot establish these cross-record invariants.

The checkpoint is payload-bearing. Metadata-only audit JSONL is separate.

Each Effect contains `effect_id`, `scope`, `operation`, `action`, `resource`,
`payload_hash` (64 lowercase SHA-256 hex characters over canonical JSON
arguments), `class`, `capability`, `idempotency_key`, `state`,
`requires_approval`, nullable `invocation_id`, and
nullable `result` (`{"payload": T}` when confirmed, including JSON null).
Effect classes are `pure`, `read`, `local_write`, `external_write`, and
`destructive`; the journal accepts the last three. States are `not_started`,
`started`, `confirmed`, `failed`, and `uncertain`. Pure/read calls are invocation
records without Effect entries. The stable key is
`awhdl:<workflow_run_id>:<effect_id>`.

`NOT_STARTED` has no invocation. `STARTED` points to a pending invocation;
`CONFIRMED` has a result. `UNCERTAIN` includes a remote-write transport failure or a
recovered in-flight write and blocks dispatch. `FAILED` denotes a trusted
reconciliation finding that the provider did not perform the action, or a
reported `local_write` error. At most one Effect exists per
(generation, operation, action, resource, payload_hash). Restore checks Effect
identity/scope, key, descriptor shape, result/state consistency and referenced
invocation before changing persisted `STARTED` to in-memory `UNCERTAIN`.
`RunStore::open` persists that recovery transition. See
[Phase 3 migration](MIGRATION_PHASE_3.md) for retry and confidentiality rules.

## Approvals (v4)

Each Effect adds `requires_approval` (boolean). The top-level `approvals` map
holds `approval_id`, `effect_id`, `scope` (only `single_action`), `action_hash`,
`content_hash` (equal to the Effect `payload_hash`), `capability`, `resource`,
`class`, `generation`, `issued_at`, `expires_at` (at most 24 h after issue),
`state` (`pending`, `granted`, `denied`, `consumed`), nullable `decided_at`
(set iff not pending) and nullable `consumed_by` (the invocation, set iff
consumed).

`action_hash` is the lowercase SHA-256 hex of the serde_json encoding (sorted
keys) of the descriptor:

```json
{"schema": "awhdl.action.v1", "workflow_run_id": "...", "correlation_id": "...",
 "generation": 0, "effect_id": "...", "operation": "...", "action": "...",
 "capability": "<capability id>#<fingerprint>", "resource": "...", "class": "...",
 "content_hash": "<payload_hash>"}
```

This encoding is deterministic for this runtime but is not claimed to be RFC
8785 JCS. `capability` is the Effect's authorizing capability handle (Phase 5). Restore recomputes every
approval hash from its Effect, rejects a mismatch, and requires that every
dispatch of an approval-bound Effect is the `consumed_by` of exactly one
approval. See [Phase 4 migration](MIGRATION_PHASE_4.md).

## Capability binding (v5)

Each Effect adds `capability`: the handle `<id>#<fingerprint>` of the capability
that authorized it, where the fingerprint is the first 16 hex characters of the
SHA-256 of the grant's permission scope (`id`, `action`, `resource`,
`constraints`, `effect_class`; not holders or revocation). Effects created
through the runtime API without a policy decision carry `unscoped`. An approval's
`capability` equals its Effect's handle, so any change to the grant's scope
changes the action hash and invalidates the approval. The same instance cannot
be re-reserved under a different handle. Grants themselves live in
configuration (`config/capabilities.toml`), not in the checkpoint. See
[Phase 5 migration](MIGRATION_PHASE_5.md).

## Compiled design

`design::CompiledDesign` (signals with classes and initial values, bound
devices, processes, assertions, cloud floor, timers, barriers, limits) is
serializable but is an in-memory program: it is rebuilt from source for every
run and is not a persisted or versioned IR in v0.1. The execution checkpoint
format is unchanged (v5).
