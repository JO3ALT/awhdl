# Runtime specification

[日本語（正本）](RUNTIME_SPEC_ja.md) | English reference translation

> **Source of truth.** Normative: [LANGUAGE_SPEC](LANGUAGE_SPEC.md),
> [RUNTIME_SPEC](RUNTIME_SPEC.md), [SECURITY_SPEC](SECURITY_SPEC.md),
> [IR_SPEC](IR_SPEC.md) and [PROFILE_v0.1](PROFILE_v0.1.md). Non-normative:
> [IMPLEMENTATION_GUIDE](IMPLEMENTATION_GUIDE.md), examples, tutorials,
> migration notes and status reports. On conflict, PROFILE_v0.1 wins, then the
> normative specs. Only PROFILE_v0.1 defines the v0.1 implementation scope.
> This English document is a reference translation; the Japanese document is
> normative and takes precedence if the versions differ.

Scope: invocation identity, generation, correlation, Value / Event / Invocation,
event delivery, barriers, cancellation, retry, checkpoint and crash recovery,
Effect semantics, completion evaluation, and AWHDL design execution. Execution semantics here take
precedence over runtime descriptions in the language and system specs.
Machine-readable formats are in [IR_SPEC](IR_SPEC.md).

## Identity and ownership

The trusted runtime generates `workflow_run_id` and `correlation_id` UUIDs once
per logical workflow input. `RunStore.run_id` is the same identifier as
`workflow_run_id`; it is not a second namespace. Multiple inputs require distinct
execution contexts. A fresh context starts at generation 0.

Each actual LLM or MCP dispatch has a fresh `invocation_id`, nullable
`parent_invocation_id`, inherited correlation/generation, UTC `created_at`, and
1-based `attempt`. Retries retain their operation slot and increment attempt;
logical revisions explicitly advance generation and reset slot attempts.
A fallback provider is another attempt. A new planner turn uses a new operation
slot (`planner:<iteration>`); malformed-output retries reuse that slot.
Action devices use `action:<configured-action-name>`. Sibling parallel branches
use distinct operation slots and inherit their parent's context. Parents must
exist in the same context and generation. Root calls have a null parent.

These fields are not AWHDL literals or planner-controlled arguments. Adapters
bind responses to the request's locally retained identity, never to fields in
untrusted output. `finish` / `finish_result` check the entire identity and require a pending
record; foreign, stale, forged, unknown and duplicate completions are rejected
before their payload can be consumed. Failures retain identity as well.

## Barrier

A barrier names required operation slots. It is ready only when every slot has
a completed invocation in the current run/correlation/generation. Missing or
failed slots do not make it ready. Foreign generations, unknown identities,
unexpected/duplicate slots and superseded attempts are errors. Mere presence of
a stored payload never makes a barrier ready.

The AWHDL `barrier` construct is executed per signal by the design interpreter
(see "AWHDL design execution"): it raises `barrier.ready` when all member
signals were written since it last fired. That interpreter barrier is
independent of the persistent API above.

## Checkpoint and recovery

`execution.json` is the authoritative execution checkpoint (format v5). Before a device future
is polled, its pending identity is written through a temporary file, synced,
renamed and the directory synced. Completion, its result and its terminal event are validated and persisted in
one checkpoint before returning a result. `state.json` also embeds the identity snapshot, but may lag
behind `execution.json` while a call is in progress. Recovery must prefer the
latter and check its run ID against the owning run directory/state.

`ExecutionState::restore` checks version and identity consistency. Pending records
remain pending: identity recovery does not reissue a device request. A later
trusted response may complete that exact identity. Identity restoration does not
replay a call or provide an exactly-once execution guarantee. A crash after
external success but before local persistence requires Effect reconciliation
as defined below; do not automatically replay the call.
The integration CLI currently has no resume command. This phase tests checkpoint
restoration as a runtime API, not end-to-end CLI resumption.

## Integration boundaries

The planner-driven engine (`aiconductor run`) is serial and runs one logical
input, generation 0. AWHDL designs run through a separate path
(`aiconductor run-design`, below) with delta cycles. Generation advancement and
barrier APIs prepare a future scheduler. All three device dispatch paths
(planner, model including fallback/polishing, MCP) pass through `RunStore::invoke`.
Model health probes and model launcher administration are control-plane operations,
not workflow device invocations. Duplicate MCP suppression does not dispatch a
new call and therefore does not allocate a device invocation.

The envelope is validated before the legacy engine consumes its payload.
Runtime invocation success denotes transport completion; existing adapter and
action checks still decide semantic success.

## Value, Event and Invocation (Phase 2)

`Value<T>` is latest state: scope, revision and payload. Assignments compare JSON
values using `serde_json::Value` equality, not source text or model similarity.
The first assignment (including null) emits `ValueChanged`; an equal assignment
in the same generation emits nothing and preserves revision. Different values
increment revision and append a changed-event payload snapshot. A new generation
has no visible values until explicitly assigned; an equal payload from an older
generation is therefore a new first assignment. Reading never consumes a value.

`Event<T>` is a one-shot occurrence with runtime-generated UUID, monotonic
sequence, scope, optional event causation and payload. Equal payloads published
twice are two occurrences. Queue history and consumption markers are distinct:
consumed history cannot trigger a process again. `next_event` selects the oldest
unconsumed event in the current scope. Trusted routing may consume a specific
ID; repeating that consumption returns no event. Unknown/stale IDs are errors.
Publishing again allocates a new identity; external ingress must preserve event
identity across redelivery instead of manufacturing a new occurrence.

`Invocation<T>` is a pending/completed/failed/cancelled lifecycle tied to one
full invocation identity. Completion never implicitly assigns a named Value.
Only pending → terminal is allowed, and only for the matching current identity.
A terminal transition appends exactly one `InvocationTerminated` event, together
with the successful result (if any), before exposing it to callers. Terminal
event payload is null; its invocation ID addresses the stored result. Cancellation
is a runtime transition, not proof of cancellation at the provider. A late result
after cancellation is rejected. The older boolean `finish` API records JSON null
on success; production adapters use `finish_result` with the serialized result.

`begin_caused` may bind an invocation to a same-scope existing event via
`causation_id`. Its terminal event inherits that cause. A parent invocation and
a causative event are distinct relationships; neither can be supplied by a model
response. Generic external publishing cannot create privileged lifecycle kinds.

## Process sensitivity and delta cycles

These are distinct semantic triggers, not synonyms:

| Intended sensitivity | Runtime trigger |
|---|---|
| `process(signal.changed)` | `ValueChanged` for the named Value |
| `process(event)` | Unconsumed external Event on the bound topic |
| `process(invocation.completed)` | `InvocationTerminated` with matching invocation and completed status |

Failed/cancelled lifecycles do not satisfy completed sensitivity. Value existence
alone never constitutes an event.

The AWHDL design interpreter (below) follows this distinction for `name` /
`name.changed` (a value change), `device.done` / `device.failed` /
`device.timeout` (a call's terminal state), timers and `barrier.ready`. These are
in-memory interpreter events, not yet connected to the persistent Event store
above. `event` declarations and the `.completed` spelling are rejected because
their syntax is open (LANGUAGE_SPEC).

Delta cycles concern deterministic propagation of changed Values within a
logical scheduling step. MCP/LLM completions, timers, approval arrivals, webhooks,
timeouts, cancellation and budget exhaustion enter through runtime events, not
through polling a retained Value or replaying a delta cycle. The persistent
store API records changed notifications in the same ordered event history,
distinguished by kind. Webhook and approval-arrival event providers, and event
subscriptions over the persistent store, remain future adapters.

## Durable delivery boundary

`RunStore` persists a cloned transition using write/sync/rename/directory-sync
before exposing it or polling a device. This includes Value assignment, event
publication, terminal invocation state and event consumption. A write failure
poisons the store for further transitions; an older in-memory snapshot must not
be used after a possibly committed write. Inspect the durable checkpoint to
recover. `ExecutionState` methods alone do not promise persistence.

Delivery is at-most-once for a retained event ID restored from the latest
checkpoint under one runtime owner. Consumption precedes returning its payload;
a crash afterwards can lose handler execution. There is no exactly-once external
effect guarantee or automatic reissue. The Effect journal below records
external write uncertainty separately. Consumed IDs and event history are
retained without garbage collection.
The store currently rewrites the full snapshot; this is a single-host prototype,
not a bounded queue or concurrent multi-process journal.

Runtime checkpoints now include Value/Event/result payloads and must retain their
workflow confidentiality. No payload is added to audit JSONL. Checkpoint
validation does not establish
classification, authorization or capability compliance.

## Effect, retry and idempotency (Phase 3)

A write is a distinct `Effect`, bound to a canonical descriptor: operation,
action, resource, class and SHA-256 hash of JSON arguments. The runtime owns
`effect_id` and stable idempotency key. The route may name a provider-supported
MCP argument for that key; planner input cannot select or override it. This is
a claim in trusted configuration, not proof of actual provider behavior.

The Effect state machine is:

```text
NOT_STARTED --durable start before dispatch--> STARTED
STARTED --confirmed response-------------> CONFIRMED
STARTED --error/timeout (remote write)---> UNCERTAIN
STARTED --reported error (LOCAL_WRITE)---> FAILED
STARTED --crash/reopen-------------------> UNCERTAIN
UNCERTAIN --confirmed evidence-----------> CONFIRMED
UNCERTAIN --proven not_found-------------> FAILED
UNCERTAIN --still uncertain-------------> UNCERTAIN
FAILED --new attempt, same descriptor----> STARTED (non-destructive only)
```

The first durable write reserves `NOT_STARTED`. The second commits `STARTED`
with a pending Invocation before polling the device. A successful normalized
response commits the invocation result and Effect confirmation in one snapshot.
For `EXTERNAL_WRITE` and `DESTRUCTIVE`, an error after dispatch is `UNCERTAIN`,
even when it looks like a timeout or a semantic failure. For `LOCAL_WRITE`, a
reported error is `FAILED`: the resource is within the orchestrator's own
environment, so a returned error is treated as definite, and retry is permitted. Transport success alone is insufficient: adapter normalization
must succeed before confirming a write. Provider success after a crash but before
local confirmation is represented by `UNCERTAIN` upon reopening the run.

`UNCERTAIN` never automatically retries. A trusted reconciler must inspect
provider evidence and report confirmed, not_found or still_uncertain. Proven
not_found allows a new attempt of a non-destructive Effect with the same key;
a destructive Effect is never automatically retried. Confirmed duplicates return
the stored Effect result without another provider call. An Effect is one exact
action instance: operation, action, resource and payload hash. A changed payload
or destination in the same operation is a new Effect with a new key, never reuse
of the old key. For `EXTERNAL_WRITE` and `DESTRUCTIVE`, no new instance is
reserved while another Effect in that operation is `STARTED` or `UNCERTAIN`,
because a variant could duplicate the unresolved write. `FAILED` means proven
absence at the provider, or a reported `LOCAL_WRITE` error; never an ambiguous
remote transport error.

`PURE` and `READ` calls may use ordinary retry policy. `LOCAL_WRITE` receives an
Effect journal; a reported error is retryable, while an in-flight write
recovered after a crash is `UNCERTAIN` and must be reconciled. `EXTERNAL_WRITE` and
`DESTRUCTIVE` additionally require a provider key argument or an explicit
manual-reconciliation route setting before initial dispatch. No broad retries
are granted merely because a tool returned an error. Action-bound approval and the
parameterized Capability model are defined in [SECURITY_SPEC](SECURITY_SPEC.md).

`RunStore::open` reads only v5, verifies the owning run ID, converts any
`STARTED` Effects to `UNCERTAIN`, and persists the result. It assumes one active
owner for a run directory. Reconciliation is exposed as a trusted runtime API,
not as planner JSON or a live provider lookup. There is no CLI resume or
interprocess run lock yet. Event consumption and external Effect execution are
not one atomic transaction; a consumed Event may still lack a completed Effect
after a crash, and recovery must inspect both records. Metadata-only effect
audit entries omit raw arguments and results. The checkpoint itself contains
results and requires workflow-data protection.

## Approval and capabilities

Action-bound approval (Phase 4) and the capability model (Phase 5) are security
semantics and are specified in [SECURITY_SPEC](SECURITY_SPEC.md). The runtime
enforces them at the transitions defined there: approval consumption with
`STARTED`, and authorization before each MCP, model, file and sandbox access.

## Completion (Phase 6)

A planner `complete` decision is a request for evaluation, never completion
itself. The runtime evaluates the run's completion policy from trusted facts
only: the action state machine, the Effect journal, and structured output of
typed adapters. The planner's JSON schema has no field for asserting facts;
unknown fields are rejected.

Conditions (`config/routing-defaults.toml`, per workflow under
`[workflows.<name>.completion]`, otherwise the top-level `[completion]`):

| Condition | Meaning | Provenance |
|---|---|---|
| `succeeded:<action>` | the state machine recorded success | deterministic for MCP routes; model-derived for model routes |
| `effects_resolved` | no Effect is `STARTED` or `UNCERTAIN`; always implied as hard | deterministic |
| `structured:<action>:<JSON pointer>=<JSON>` | the action's latest typed adapter `structured` value equals the literal | deterministic |
| `planner_claim` | the planner asked to complete | model-derived |

Rules:

- `hard` conditions must be deterministic. A model-derived condition in `hard`
  is a configuration error. `soft` may hold any condition.
- `safety_critical = true` requires at least one explicit hard condition. A
  workflow containing a step whose class ceiling is `external_write` or
  `destructive` must be safety-critical. LLM-only completion is therefore
  possible only for workflows that cannot affect external safety.
- Results: `COMPLETE`; `INCOMPLETE` (a hard condition is unmet but reachable;
  the planner receives `COMPLETION_REJECTED` with the unmet list and the loop
  continues); `BLOCKED` (an Effect is unresolved; the run stops for
  reconciliation); `FAILED` (a hard `succeeded`/`structured` action failed and
  exhausted its attempts); `REQUIRES_REVIEW` (hard conditions hold, soft do not,
  and `on_soft_failure = "requires_review"`, the default). `on_soft_failure` may
  instead be `retry` (treated as `INCOMPLETE`) or `complete`.
- `REQUIRES_REVIEW` returns the answer prefixed with `[REQUIRES_REVIEW]` and
  the unmet soft conditions, and the run phase is `requires_review`.
- Every evaluation is audited as `completion_evaluated`; `run_completed`
  records the final status. The iteration budget bounds repeated rejected
  completions.

The final answer text is still model prose. Completion guarantees that the
required deterministic steps happened, not that the prose describes them
correctly; a deterministic report formatter remains future work.

## AWHDL design execution

`aiconductor run-design <file> --input name=value` compiles and runs a design.

Compilation parses, runs the static checker (any diagnostic is fatal) and binds
each device to a configured route through `generic (route => "...")`:

- kinds: `agent` binds a model route or a Codex route; `mcp` binds a non-Codex
  MCP route; `deterministic` binds a non-Codex MCP route whose class is not a
  write. Other kinds are rejected.
- The device's declared location (default `local`) must equal the route's
  `location` (default `local`), so the static flow check reasons about the real
  destination. Codex routes are `cloud`.
- The planner route cannot be bound. A design needs exactly one architecture.
- Budget limits `iterations`, `wall_time`, `tool_calls` and `model_calls` are
  enforced and can only lower the configured limits; other limit names are a
  compile error. A timer requires an `iterations` or `wall_time` limit.

Execution uses delta cycles:

1. Input ports are assigned and raise `name` and `name.changed` events.
2. Each delta runs, in source order, every process whose sensitivity names an
   event of the delta. Processes read the values of the start of the delta.
3. Assignments and device results are applied at the end of the delta. Two
   processes writing different values to one signal in one delta is an error.
   A changed value raises `name` / `name.changed`; a barrier raises
   `barrier.ready` when all members were written since it last fired.
4. Concurrent `assert always/never` are evaluated after each delta; a violation
   fails the run. A sequential `assert` fails the run immediately.
5. With no pending events, the next timer tick is awaited; with no timers the
   run is `quiescent`. A budget ends the run as `exhausted`.

A device call runs the bound route through the engine's normal path (capability
authorization, Effect journal, approval, audit). Success raises `device.done`
and writes the output; failure raises `device.failed` and writes nothing; a call
`timeout` also raises `device.timeout`. A process `timeout` discards the body's
writes and runs `on timeout`. `parallel` results commit together; v0.1
dispatches its calls sequentially. A run completes only with every Effect
resolved. Signal values are kept in memory; audit records signal names and
classes, never values.
