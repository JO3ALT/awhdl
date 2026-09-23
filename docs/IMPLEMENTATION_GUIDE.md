# Implementation guide

> **Non-normative.** This guide describes how the implementation is built. It
> defines no language, runtime or security semantics and no scope. Normative:
> [LANGUAGE_SPEC](LANGUAGE_SPEC.md), [RUNTIME_SPEC](RUNTIME_SPEC.md),
> [SECURITY_SPEC](SECURITY_SPEC.md), [IR_SPEC](IR_SPEC.md) and
> [PROFILE_v0.1](PROFILE_v0.1.md), which alone defines v0.1 scope.

## Scope discipline for contributors and coding agents

1. Read PROFILE_v0.1 first. Implement only features whose profile is `v0.1`.
2. Do not implement a `v0.2` feature, or anything the profile does not list,
   even if a spec or older brief describes it. Propose a profile change with a
   DESIGN_DECISIONS entry first.
3. When a change alters a feature's status, update its matrix row and Evidence
   in the same change. `profile_matrix_is_backed_by_existing_tests` fails if a
   claimed implementation names no existing test.
4. Resolve an ambiguity by recording the decision and its reason in
   [DESIGN_DECISIONS](REFINEMENT_DECISIONS_ja.md), not by silent interpretation.
5. Never make LLM or agent output part of the trusted control plane.

## Rust workspace (`awhdl/`)

| Directory | Package (binary) | Role |
|---|---|---|
| `awhdl-ast` | `awhdl-ast` | AST types |
| `awhdl-parser` | `awhdl-parser` | pest grammar (`awhdl.pest`) and parser |
| `awhdl-checker` | `awhdl-checker` | structural static checks (`AWHDL-E2xx`) |
| `conductor-cli` | `conductor-cli` (`aic`) | `aic check` for AWHDL sources |
| `aiconductor-runtime` | `aiconductor` | Tokio runtime: execution state, dataflow, Effects, approvals, capabilities, completion, engine, MCP/LLM clients, CLI |

Rust edition 2024, async runtime Tokio, CLI parsing clap, parser pest.

## Test strategy

- Required for every change, from `awhdl/`:
  `cargo test --workspace --offline`, `cargo fmt --check`,
  `cargo clippy --workspace --all-targets --offline -- -D warnings`.
- Unit tests call no live provider. Semantics are tested at the
  `ExecutionState` API level and through `RunStore` with temporary directories.
- Production configuration is loaded and validated by config tests, so an
  inconsistent route, capability or completion policy fails the suite.
- Checkpoint formats are validated against `docs/schema` using the offline
  `dataflow_checkpoint` example.
- Live checks are examples, not tests: `live_matlab_smoke` exercises the MATLAB
  MCP through the engine's adapter, capability and Effect path, and
  `aiconductor run-design ../examples/awhdl/matlab_sum.awhdl --input task=...`
  runs a compiled AWHDL design against MATLAB.
- Interpreter semantics are tested with a scripted `Dispatcher`; approval flow
  with a scripted `Approver`; HumanPort's approver mode by serving the real
  script's MCP side without its GUI.

## Historical briefs

`CODEX_IMPLEMENTATION_INSTRUCTIONS.md` (root) and
`awhdl/IMPLEMENTATION_GUIDE_v0.2*.md` are earlier implementation briefs. Their
milestone and scope statements are superseded by PROFILE_v0.1.
