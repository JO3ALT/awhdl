# Phase 3 migration and verification

> Superseded for the checkpoint format by [Phase 4](MIGRATION_PHASE_4.md) (v4).

Phase 3 adds a durable Effect journal and write retry rules. The authoritative
checkpoint is now [`execution-v3.schema.json`](schema/execution-v3.schema.json).
Formats v1/v2 remain historical; `ExecutionState::restore` rejects them. They
cannot be upgraded by inventing Effect records because an already-sent write
may be impossible to distinguish from an unstarted write. Keep old runs for
inspection and start a new run explicitly. `RunStore::open` only accepts v3 and
checks the checkpoint's run ID against its directory and `state.json` when present.

Every MCP route should declare `effect_class` (`pure`, `read`, `local_write`,
`external_write`, `destructive`). An omitted MCP class is treated as
`external_write`; omitted model class is `pure`. External/destructive routes
need either `idempotency_argument` supported by their provider or explicit
`manual_reconciliation = true`. The latter allows first dispatch and requires
trusted reconciliation before retry after an ambiguous outcome. The supplied
argument name is configuration owned; a planner-supplied key is rejected.
The example routing file declares classes for its existing actions. MATLAB
(`calculation_graphing`) and KDB (`table_analysis`) are `local_write`: they run
on lab-owned hosts, and a returned code error must not block the planner's
correct-and-rerun loop. Codex-backed `open_data_acquisition` remains
`external_write` with manual reconciliation. The declared
class is a trusted configuration claim, not a tool sandbox or proof of provider
behavior. Capability enforcement remains Phase 5.

`EffectRequest` hashes the canonical JSON arguments with SHA-256, excluding the
orchestrator-generated provider key. The journal retains action, resource,
class, payload hash, invocation ID and stable idempotency key; it does not add
raw arguments to audit JSONL. The invocation result and Effect confirmation
result are stored in the checkpoint and inherit workflow-data confidentiality.
The key is `awhdl:<run UUID>:<effect UUID>` and is reused for that exact Effect
following a proven `not_found` reconciliation. A changed payload or resource
in one operation/generation is a new Effect with a new key. For external and
destructive classes, that new instance is rejected while another Effect in the
same operation is `STARTED` or `UNCERTAIN`. A changed class for the same
instance is rejected.

`NOT_STARTED` is durable before a call is prepared. The matching invocation and
`STARTED` are committed together before the device future is polled. On normal
success, invocation completion and `CONFIRMED` are committed together; a
confirmed duplicate returns the saved result. For external/destructive writes, any transport/semantic error, provider
timeout, or serialization failure after dispatch becomes `UNCERTAIN`. For
`local_write`, the same reported error becomes `FAILED` and may be retried.
`RunStore::open` converts a persisted `STARTED` to `UNCERTAIN` and persists that
transition before allowing any calls. An `UNCERTAIN` Effect is never blindly
retried, even when a provider key exists.

Trusted `reconcile_effect` accepts `confirmed(result)`, `not_found`, or
`still_uncertain`, based on provider evidence. `not_found` permits a retry of a
non-destructive Effect with the original key. Destructive retry remains forbidden.
`RunStore::open` and reconciliation are runtime APIs; CLI resume and automatic
provider lookup are not implemented. A `confirmed` reconciliation after a
transport failure leaves that attempt's invocation failed while recording the
provider's confirmed Effect result. Consumers should use the Effect result for
this case, rather than treating the failed transport as a successful invocation.

Current serial engine MCP calls use this journal for configured write classes;
model and read-only MCP calls continue to use ordinary invocation semantics.
The in-memory `successful_mcp_calls` filter remains an optimization. Only the
journal provides cross-restart deduplication of a confirmed Effect. An existing
MCP's declared `read` class is a configuration assertion; review it if its
allow-listed tool surface changes. `manual_reconciliation` is not human approval.
Action-bound approval and capability checks are future phases.

Required verification from `awhdl/`: `cargo test --workspace --offline`,
`cargo fmt --check`, and
`cargo clippy --workspace --all-targets --offline -- -D warnings`.
The offline `dataflow_checkpoint` example emits a v3 checkpoint for schema
validation. No live provider call is required for these tests.
