# Phase 2 migration and boundaries

Historical Phase 2 record. Phase 3 supersedes checkpoint format v2 and adds
the Effect journal; see [Phase 3 migration](MIGRATION_PHASE_3.md).

Phase 2 separates latest values, one-shot events and invocation lifecycles in
`aiconductor-runtime`. The AWHDL parser/AST still implement Milestone 1.
Dotted sensitivity names already parse as untyped strings and receive only
root-name structural checking; parsing them does not establish trigger semantics.
New `event` declarations remain unsupported.

- New checkpoints use `execution-v2.schema.json`. Version 1 is retained as a
  historical schema; `ExecutionState::restore` rejects it. There is no automatic
  migration: v1 lacks event identities, consumption history and result payloads.
  Keep old runs for inspection; start new runs explicitly. Do not fabricate
  consumed events or resend pending calls to reconstruct missing state.
- `Invocation<T>` replaces the concrete record structure; `InvocationRecord`
  aliases its JSON specialization. `ResultEnvelope<T>` remains the adapter return
  type. `RunStore::invoke` now requires `T: Serialize` and checkpoints successful
  results before returning. Existing planner/model/MCP call sites meet this bound.
- Successful JSON null is encoded as `result: {"payload": null}`; missing results
  are `result: null`. Pending/failed/cancelled invocations have no result.
- `execution.json`, and snapshots embedded in `state.json`, now contain payloads.
  They must be handled as workflow data with the input/output's confidentiality,
  not as metadata-only audit logs. No new payload fields are added to audit JSONL.
  Existing classification/capability enforcement is not expanded by this phase.
- Use `RunStore::assign_value`, `publish_event`, and `consume_event` for durable
  changes. The corresponding `ExecutionState` APIs only mutate memory. Restoring
  from an old snapshot can replay old work: always restore the authoritative
  `execution.json`, check the owning run ID, and allow only one owner per run.
- Consumption is persisted before delivery (at-most-once). A crash immediately
  afterwards can lose handling. This does not promise exactly-once side effects,
  atomic event-handler execution, external ingress deduplication or reconciliation.
  These require the Phase 3 effect protocol. Do not truncate consumed-event history.
- A persistence error makes the `RunStore` refuse further transitions/dispatch.
  Inspect and restore the durable checkpoint; no automatic retry or CLI resume is
  provided. Cancellation changes runtime state only, not a running provider task.
- Current serial engine completion paths persist invocation results and lifecycle
  events. They still use the synchronous return envelope; they do not schedule
  AWHDL processes from the queue. Value propagation, timer/webhook/approval
  adapters, subscriptions, parallel scheduling and delta-cycle execution are not
  implemented here. Generic trusted ingress can publish external events.

Regression coverage includes unchanged-value assignment, distinct equal events,
consumed-event restart, invocation-specific completion, JSON-null results,
cancellation, stale-generation isolation, corrupt-checkpoint rejection and
write-failure handling. Required checks run from `awhdl/`:

```sh
cargo test --workspace --offline
cargo fmt --check
cargo clippy --workspace --all-targets --offline -- -D warnings
```

`cargo run -p aiconductor --example dataflow_checkpoint --offline` emits a
representative v2 checkpoint without contacting devices. Validate its output
against `schema/execution-v2.schema.json` with a Draft 2020-12 validator.
See progress for verification results.
