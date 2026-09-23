# AWHDL design decisions

[日本語（正本）](DESIGN_DECISIONS_ja.md) | English reference translation

> The Japanese document is normative and takes precedence if the versions differ.

## Formal name

AWHDL formally stands for **Agentic Workflow Harness Description Language**.
The H means Harness, not Hardware. Harness expresses the design intent of
connecting multiple AIs, MCPs, deterministic tools, and humans while binding
their data flow, events, iterations, budgets, permissions, completion rules,
and safety boundaries into one checkable and executable structure.

VHDL inspired concurrency, signals, processes, events, and structural checks,
but AWHDL does not describe electronic circuits and does not target hardware
synthesis.

## Milestone 1 scope

Milestone 1 implements only the syntax needed for the canonical `hello.awhdl`
vertical slice: entities and ports, architectures, simple device and signal
declarations, processes, device calls, signal assignments, and `null`.

The AST already keeps data-classification spelling and source byte spans so the
Milestone 2 checker can add the classification lattice without replacing the
parser boundary.

## Parsing and diagnostics

The parser uses Pest and accepts lowercase AWHDL keywords. AWHDL is not treated
as fully VHDL-compatible. Syntax outside the implemented subset is rejected as
`AWHDL-E101`; it is never silently ignored.

The checker performs name and structural checks only. Its `E2xx` diagnostics do
not imply that port typing or security-flow analysis has been implemented.

## Integration runtime boundary

The `aiconductor-runtime` integration prototype remains independent and is not
part of the normative AWHDL compiler pipeline. Milestone 1 does not compile
AWHDL into runtime configuration or execute an AWHDL design. Integration
belongs after a versioned IR is introduced.

## Deferred questions

- Keyword case sensitivity and Unicode identifier rules.
- Full expression grammar and overload resolution.
- Closing-name equality for entities and architectures.
- Multiple drivers and signal resolution.
- Generic maps, device port declarations, `if`, `case`, timers, budgets, and
  parallel blocks.
- Classification lattice, taint, declassification, and cloud-export policy.

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

## v0.1 backlog implementation (2026-09-23)

v0.1 syntax, static information-flow checks, compilation to the runtime with
route binding, delta-cycle execution, the HumanPort approver adapter and MATLAB
file execution are implemented. Status is in
[PROFILE_v0.1](PROFILE_v0.1.md); decisions in
[design decisions](REFINEMENT_DECISIONS_ja.md).
