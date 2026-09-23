# Security specification

[日本語（正本）](SECURITY_SPEC_ja.md) | English reference translation

> **Source of truth.** Normative: [LANGUAGE_SPEC](LANGUAGE_SPEC.md),
> [RUNTIME_SPEC](RUNTIME_SPEC.md), [SECURITY_SPEC](SECURITY_SPEC.md),
> [IR_SPEC](IR_SPEC.md) and [PROFILE_v0.1](PROFILE_v0.1.md). Non-normative:
> [IMPLEMENTATION_GUIDE](IMPLEMENTATION_GUIDE.md), examples, tutorials,
> migration notes and status reports. On conflict, PROFILE_v0.1 wins, then the
> normative specs. Only PROFILE_v0.1 defines the v0.1 implementation scope.
> This English document is a reference translation; the Japanese document is
> normative and takes precedence if the versions differ.

Scope: trust principles, classification, taint, cloud egress, declassification,
capabilities, human approval and audit. Whether each item is required in v0.1,
and its implementation status, is recorded only in [PROFILE_v0.1](PROFILE_v0.1.md).

## Trust principles

- LLMs and agents are never the trusted control plane. Policy, scheduling,
  sandboxing, deterministic checks, budgets and human approval are trusted.
- An agent cannot choose its own identity, classification, capability,
  approval, Effect identity or completion status. Planner output that tries to
  is rejected (see the runtime's closed decision schema).
- Everything not granted is denied.

## Classification, taint, cloud egress and declassification

The target semantics are specified in the AI Conductor system specification,
[`AI_CONDUCTOR_SYSTEM_SPEC_v0.2_ja.md`](AI_CONDUCTOR_SYSTEM_SPEC_v0.2_ja.md)
(Japanese normative; English reference translation alongside), sections 5
(trust zones) through 11 (defense in depth). They are normative for these topics.
`SECURE_ORCHESTRATOR_SPEC_v0.1.md` is historical design input.

For v0.1, PROFILE_v0.1 requires the classifications `public`, `internal` and
`restricted`, the locations `local` and `cloud`, and a static rule that
`restricted` data never flows to a `cloud` location. Declassifiers, taint
through control flow, and cloud DLP / egress inspection are v0.2.

### v0.1 flow rules (implemented)

- A value's class is its declared label; an unlabeled type is `restricted`.
  An expression has the strongest class of the signals it reads; literals and
  device, timer or barrier events are `public`.
- `AWHDL-E301`: data at or above the cloud floor must not be an argument to a
  device whose location is `cloud`. The floor is `restricted`, lowered by
  `assert never (<class> -> cloud);`.
- `AWHDL-E302`: argument class must not exceed a device's declared `clearance`.
- `AWHDL-E303`: a device result keeps the strongest class of its arguments, and
  an assignment keeps the class of its value; storing either in a lower-labeled
  signal is an implicit declassification and is rejected. Without this, data
  could be laundered through a local device into a lower label.
- A device without a `location` is `local`; binding rejects a local device bound
  to a cloud route. Route locations are configuration (`location = "cloud"`).
- At runtime the same cloud-floor and clearance checks run again before each
  device call (defense in depth), from the declared classes of the arguments.
- Not covered in v0.1: implicit flows through `if` conditions, and taint across
  processes beyond declared labels (each store is checked against its label).

## Action-bound approval

Approval is never an abstract boolean. It authorizes one exact Effect instance
in one generation. The runtime builds a canonical action descriptor from the
Effect record and the current scope (see [IR_SPEC](IR_SPEC.md#approvals-v4)) and
binds the approval to its SHA-256 `action_hash`. Changing the payload,
destination/resource, commit or diff (both are payload), generation, capability
or class changes the hash, or yields a different Effect, so no earlier approval
applies.

```text
reserve Effect (NOT_STARTED)
  -> request_approval(effect, exact parameters)   -> PENDING
  -> Human device shows display text + canonical action
  -> decide_approval(id, echoed action_hash)      -> GRANTED / DENIED
  -> begin + start_effect: recompute hash, check expiry,
     GRANTED -> CONSUMED and Effect -> STARTED in one checkpoint
```

- `request_approval` requires the exact dispatch parameters; their hash must
  equal the Effect content hash. The Human request carries both a non-authoritative
  `display` string and the authoritative `canonical_action` (descriptor plus
  parameters). Display text alone is never approval evidence.
- A decision must echo the `action_hash` of the request, arrive before
  `expires_at`, and the Effect must still be current. Only trusted adapter code
  can call it; there is no planner path.
- An approval is consumed by exactly one dispatch. A retry after `not_found`
  reconciliation, or any other re-dispatch, needs a new approval (replay
  protection). At most one live (pending or granted, unexpired) approval exists
  per Effect.
- A new generation makes earlier Effects stale, so their approvals cannot be
  decided or consumed; the same action in the new generation is a new Effect.
- Scope vocabulary is `single_action`, `transaction` and
  `time_limited_session`. This runtime accepts only `single_action`; the broader
  scopes are rejected as not permitted by policy.
- Route setting `human_approval` (`required` / `not_required`) defaults to
  required for `external_write` and `destructive`, not required otherwise.
  Destructive routes cannot opt out, and non-write routes cannot set it. This is
  distinct from the Codex-specific `approval_policy`.
- Audit JSONL records `approval_requested` (IDs, hash, expiry),
  `approval_decided` (state) and the consumed `approval_id` on `effect_started`.
  Parameters are not written to audit.

The Human approval adapter is described under "Human approval adapter" below;
without it, approval-bound writes fail closed.

## Capability model

Tool, filesystem, sandbox, network and model permissions are one model,
declared in `config/capabilities.toml`:

```text
Capability { id, subjects, action, resource = kind:pattern, constraints, effect_class, revoked }
authorize(subject, CapabilityRequest { action, resource, params }) -> { id, handle, effect_class }
```

- Default deny. A request is permitted only by a non-revoked grant held by the
  subject (route name), with the same action, a matching resource pattern
  (`**` spans segments, `*` stays in one), and constraints that cover every
  request parameter and are all supplied. Unlisted parameters are denied.
- Resources: `mcp:<server>/<tool>`, `file:<project-relative path>`,
  `model:<profile>`, `host:<name>` and domain kinds such as `repo:<name>`.
  `file:` resources are produced by canonicalizing the path, resolving symlinks;
  a path outside the project root is denied before matching.
- When several grants match, the most severe `effect_class` wins, so a broad
  overlapping grant cannot understate risk. Ties resolve to a fixed config order.
- The authorizing grant's `effect_class` drives Effect journaling (Phase 3) and
  the default approval requirement (Phase 4) for that call, so classes are
  per tool rather than per route. A route's `effect_class` is a ceiling: a grant
  above it fails configuration validation, which keeps the route-level
  idempotency/reconciliation/approval checks sound.
- Handles (`id#fingerprint`) change whenever a grant's scope changes; Effects and
  approvals bind to them (see [IR_SPEC](IR_SPEC.md#capability-binding-v5)).
- `Capability::narrow` derives a sub-capability: same action, a resource within
  the parent (conservative containment), the same constraint keys with each
  value equal or a literal the parent pattern matches, and a class no higher.
  Revoked grants deny and cannot be narrowed.

Enforcement points in the engine (planner runs and design runs):

| Access | Request | Where |
|---|---|---|
| MCP tool | `mcp.call` on `mcp:<server>/<tool>` | before dispatch, after adapter preparation |
| Planner-supplied file argument (Lean, Prolog, MATLAB file tools) | `file.read` on `file:<path>` | adapter preparation |
| Codex sandbox | `sandbox.read_only` / `sandbox.workspace_write` / `sandbox.full_access` on `host:local` | Codex adapter |
| Codex network | `network.connect` on `host:*` (full access always implies it) | Codex adapter |
| Model | `model.chat` on `model:<profile>` for each candidate and the planner | before model start |

Configuration load authorizes every route's static needs (tools, models,
sandbox, network), rejects grants naming unknown routes, and rejects grants
above a route's class. `mcp.call` authorizations are audited as
`capability_authorized` with the handle and class; a denial fails the action
with a `capability denied` error. Agents receive no credentials: SSH keys and
CLI logins stay in launcher scripts, and a route only holds handles.

Not covered: data classification and location arguments of the conceptual
`authorize(subject, capability, resource, classification, location, context)`
(the cloud-export gateway is not implemented); argument contents of passthrough
tools (KDB, filter), which remain restricted by their MCP servers' own roots;
per-host network control inside Codex, which can only be all-or-nothing; and a
credential broker.

## Audit

- Audit JSONL is metadata only: identifiers, operation and action names,
  resources, classes, states, hashes, capability handles and budget usage.
  It never contains raw tool arguments, approval parameters, model prompts or
  results.
- The execution checkpoint (`execution.json`) is payload-bearing. It stores
  results, Values, Event payloads and Effect results, inherits the
  confidentiality of the workflow data, and must be protected accordingly.
- Security-relevant decisions are audited: `capability_authorized`,
  `approval_requested`, `approval_decided`, `effect_started` (with the consumed
  approval), `effect_finished`, `effect_reconciled` and `completion_evaluated`.
- Audit completeness is not a security control by itself: a denial is recorded
  as the failure of the action that requested it.

## Completion trust boundary

Completion is evaluated by the runtime from trusted facts, and model-derived
signals can never satisfy a hard condition. The rules are in
[RUNTIME_SPEC](RUNTIME_SPEC.md#completion-phase-6).

## Human approval adapter

`[approval]` in `config/runtime.toml` names the approval MCP server (HumanPort
in approver mode, `scripts/mcp/start-humanport-approver`). For an
approval-bound write the engine reserves the Effect, creates the approval with
the exact dispatch arguments, and asks the human. The HumanPort task carries the
canonical action and `action_hash`; the answered task must echo that hash, and
the decision is recorded before dispatch. A denial, a mismatched hash, an
expired request or an adapter error leaves the Effect `NOT_STARTED` and the
provider uncalled.

- Approver mode removes `human.answer`, `human.cancel` and `human.list` from the
  MCP surface; only the GUI answers. The adapter refuses a server that still
  exposes answering.
- No route may use the approval server (configuration validation), so neither
  the planner nor an agent route can reach it.
- Without `[approval]`, approval-bound writes fail closed as before.
