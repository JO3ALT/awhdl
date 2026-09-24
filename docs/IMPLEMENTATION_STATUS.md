# AWHDL implementation status

> **Scope and status source of truth (Phase 7, 2026-09-23):** The v0.1 scope
> and each feature's status are defined only by the matrix in
> [PROFILE_v0.1](PROFILE_v0.1.md). This document summarizes it; the
> matrix wins on any difference. This document defines no scope of its own.

[日本語（正本）](IMPLEMENTATION_STATUS_ja.md) | English reference translation

> The Japanese document is normative and takes precedence if the versions differ.

## Release state (2026-09-23)

- Specification: draft v0.2; implementation scope: PROFILE_v0.1.
- Compiler: parser, static checker (structure `E2xx`, information flow `E3xx`,
  profile `E4xx`) and compilation to the Rust runtime with route binding.
- CLI: `aic check <file>`; `aiconductor run-design <file> --input name=value`.
- Runtime: delta-cycle execution of processes, timers, timeouts, parallel
  blocks, barriers, budgets and assertions; device calls through the engine's
  capability, Effect, approval and audit path.

## Implemented syntax

- entities, ports, architectures;
- devices of kind `agent`, `mcp`, `deterministic` with `generic (...)`
  (`route`, `location`, `clearance`);
- signals with classification labels and constant initial values;
- `timer`, `budget`, `barrier` declarations;
- `assert always (...)`, `assert never (...)`, `assert never (<class> -> <location>)`;
- processes with sensitivity lists, process `timeout` and `on timeout`;
- device calls with optional `timeout`, assignments, `if`/`elsif`/`else`,
  `parallel`, sequential `assert`, `null`;
- expressions with `or`, `and`, `not`, comparisons, integer `+`/`-`, literals
  and selected names.

## Implemented checks

- parse errors with line and column; reserved keywords;
- duplicate and unknown names, writes to input ports, budget and timer limits,
  barrier members, parallel write conflicts;
- classification and location names limited to the v0.1 profile;
- restricted-to-cloud flow (`E301`), clearance (`E302`) and implicit
  declassification (`E303`); unlabeled data is treated as restricted;
- at compilation: device kind, route binding and location consistency.

## Not implemented

- Event declarations and completion syntax (semantics exist; syntax is open);
- `case`, `await`, FSM types, `retry`, `approve`, `policy`, `export`,
  `configuration`, temporal assertions, testbenches and fault injection;
- declassifiers, confidential/secret classes, sandbox/private_cloud/external
  locations, implicit flows through conditions;
- persisted versioned program IR; concurrent dispatch inside `parallel`;
- `aic compile`, `sim`, `graph`, and `audit`.

Unsupported syntax must produce a diagnostic rather than being ignored.

## Conformance policy

The specification defines the intended language. Only features listed under
“Implemented syntax” and “Implemented checks” are currently supported by the
prototype. Future milestones should update this document in the same change as
their implementation and tests.

## Phase 1 refinement (2026-09-23)

Integration runtime identity, checkpoint and barrier APIs are implemented separately
from the AWHDL compiler. See [decisions](REFINEMENT_DECISIONS_ja.md) and
[migration / limits](MIGRATION_PHASE_1.md). This does not add AWHDL
compile/run, signal scheduling or a CLI resume command. Workspace verification
passed on 2026-09-23: 33 tests, format check and Clippy with warnings denied.

## Phase 2 refinement (2026-09-23)

The integration runtime now separates `Value<T>`, `Event<T>` and `Invocation<T>`.
Checkpoint v2 stores latest values, event history/consumption and invocation
results. All device completions create lifecycle events; the serial engine still
consumes return envelopes. No AWHDL scheduler, delta-cycle execution, new source
syntax or CLI resume is claimed. See [runtime](RUNTIME_SPEC.md),
[decisions](REFINEMENT_DECISIONS_ja.md),
[migration](MIGRATION_PHASE_2.md), and
verification.

## Phase 3 refinement (2026-09-23)

Write-class MCP routes now use the durable Effect journal. Timeout/unknown
outcomes of external/destructive writes block retry until trusted
reconciliation; a reported `local_write` error (MATLAB, KDB) is FAILED and
retryable. Confirmed duplicates reuse stored results. Checkpoint v3 records Effect identity and state. Existing parser,
AST and structural checker are unchanged. See [runtime](RUNTIME_SPEC.md),
[design decisions](REFINEMENT_DECISIONS_ja.md),
[migration](MIGRATION_PHASE_3.md), and
verification.

## Phase 4 refinement (2026-09-23)

Write Effects can require human approval bound to the SHA-256 of one canonical
action instance. Approvals are single-use, expire, and are consumed with
`STARTED` in one checkpoint. External/destructive writes require approval by
default; `open_data_acquisition` explicitly opts out. No approval adapter is
connected, so required approvals fail closed. Checkpoint v4. Parser, AST and
structural checker are unchanged. See [runtime](RUNTIME_SPEC.md),
[design decisions](REFINEMENT_DECISIONS_ja.md),
[migration](MIGRATION_PHASE_4.md), and
verification.

## Phase 5 refinement (2026-09-23)

Tool, file, sandbox, network and model permissions are one default-deny
capability model (`config/capabilities.toml`) with a single `authorize`.
The authorizing grant's class sets journaling/approval per tool; route classes
are ceilings. Planner file paths are canonicalized and limited to granted
project paths. Effects and approvals bind to capability handles. Checkpoint v5.
Parser, AST and structural checker are unchanged. See
[runtime](RUNTIME_SPEC.md), [design decisions](REFINEMENT_DECISIONS_ja.md),
[migration](MIGRATION_PHASE_5.md), and
verification.

## Phase 6 refinement (2026-09-23)

A planner completion claim only requests evaluation; the runtime decides from
the state machine, Effect journal and typed adapter output. Hard conditions are
deterministic, model signals are soft, and external workflows are
safety-critical. Configuration only; checkpoint stays v5. Parser, AST and
structural checker are unchanged. See [runtime](RUNTIME_SPEC.md),
[design decisions](REFINEMENT_DECISIONS_ja.md),
[migration](MIGRATION_PHASE_6.md), and
verification.

## Phase 7 v0.1 profile (2026-09-23)

Documents were reorganized by role. The v0.1 scope and every feature's status
now live only in the matrix of [PROFILE_v0.1](PROFILE_v0.1.md), which
a test keeps consistent with the code. This file summarizes it. Normative:
LANGUAGE_SPEC, RUNTIME_SPEC, SECURITY_SPEC, IR_SPEC, PROFILE_v0.1. See
[design decisions](REFINEMENT_DECISIONS_ja.md) and
[migration](MIGRATION_PHASE_7.md).

## Operational additions (2026-09-24, outside the profile, non-normative)

For AI Conductor operations the runtime gained the following; the language
syntax and the PROFILE_v0.1 matrix are unchanged.

- Launch profiles on hosts without `ssh_target` start locally; SSH tunnels
  have a connect timeout.
- The planner falls back through the `loop` route's fallback models.
  `aiconductor run --controller <profile>` uses only that model, and
  `--decider on|off` toggles the decider.
- Optional decider (`[decider]` in `runtime.toml`, needs the `model.decide`
  capability): a System One style decision model picks the next action with
  probabilities and the LLM only fills its arguments; low confidence hands
  the decision back to the LLM.
- Routes with `location = "cloud"` are offered to the planner only when the
  instruction contains `cloud_opt_in_marker`; workflows are unaffected.
- Adapters: Lean, Prolog and MATLAB project files are read by the runtime
  and sent as code; arguments are recovered from backtick code in the
  instruction; new KDB (`run_q`) and Filter (`list_files`, `preview_file`)
  adapters; Japanese prose is never sent as code.
- The AI Conductor controller benchmark (`bench/controller/`) led to the 27B
  model as the default controller.
