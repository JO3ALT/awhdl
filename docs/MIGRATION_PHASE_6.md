# Phase 6 migration and verification

[日本語（正本）](MIGRATION_PHASE_6_ja.md) | English reference translation

> The Japanese version is canonical. This English version is a reference
> translation; where they differ, the Japanese version prevails.

Phase 6 separates deterministic (hard) completion conditions from
model-generated (soft) signals. The checkpoint format is unchanged (v5);
completion policy lives in configuration and each evaluation is audited.

## Behavior changes

- A planner `complete` no longer ends the run by itself. The runtime evaluates
  the completion policy; an unmet hard condition returns `COMPLETION_REJECTED`
  to the planner and the loop continues within the iteration budget.
- Every run implicitly requires `effects_resolved`: a run with an `UNCERTAIN`
  or `STARTED` Effect ends as `BLOCKED` instead of completing.
- `open_data_population` is safety-critical and requires its three steps to
  succeed. `cvim_full_loop` requires its six MCP steps; its model step
  (`initial_coding`) is soft, and failing it yields `REQUIRES_REVIEW`.
- Free-form runs (no workflow) use the top-level `[completion]` policy, which
  is empty by default: completion needs only resolved Effects. This keeps
  today's behavior for research/Q&A runs, which cannot trigger external writes
  through completion.
- Configuration load rejects model-derived hard conditions, unknown actions in
  conditions, and non-safety-critical workflows with external/destructive steps.
- The planner prompt states that completion is checked and may be rejected.

## Limits

- The final answer is still LLM prose; hard conditions do not verify it.
- `structured:` conditions read the latest structured output per action from
  the current process; they are not restored from a checkpoint (no CLI resume).
- Conditions are configuration syntax; the AWHDL `completion when hard {...}
  soft {...}` source syntax is not parsed yet (semantics fixed first, as the
  procedure allows).

## Verification

From `awhdl/`: `cargo test --workspace --offline`, `cargo fmt --check`, and
`cargo clippy --workspace --all-targets --offline -- -D warnings`. No schema
change and no live provider are required.
