# Phase 1 migration and verification

Historical Phase 1 record. Phase 2 supersedes the checkpoint format and adds
payload-bearing dataflow storage; see [Phase 2 migration](MIGRATION_PHASE_2.md).

- Root v0.1 specs and all eight language/system v0.2 copies link to the
  normative execution-identity addendum. File reads succeeded on resumption.
- Existing AWHDL source and parser/AST are unchanged. No new syntax is silently
  accepted. The structural checker remains structural only.
- New runs allocate correlation and invocation metadata. Audit gains
  `invocation_started` / `invocation_finished` records with identity and outcome.
- `state.json` now requires `execution`; the newer `execution.json` stores the
  authoritative identity state after every dispatch/completion. Old checkpoints
  lacking identity must not be resumed by inventing IDs. Preserve them for audit
  and start a new run only through an explicit caller decision.
- `ExecutionState::restore` recovers identity without issuing calls. No CLI resume,
  AWHDL compile/run, full signal store or parallel scheduler is claimed.
- Regression tests cover stale completion, retries, sibling scope, mixed-generation
  barrier rejection, forged/replayed identity, checkpoint restore, pre-dispatch
  persistence, audit metadata and error/retry identity.
- Required verification (from `awhdl/`): `cargo test --workspace`,
  `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`.

The procedure is being applied one phase at a time. Phase 2–7 remain pending.

Verification on 2026-09-23 after resumption: all 33 workspace tests passed
(including 29 runtime tests), workspace format check passed, and workspace
Clippy passed with warnings denied. Test and Clippy used `--offline`.
The earlier fixture read failure did not recur; its underlying cause is unconfirmed.
Phase 1 verification is complete within the implementation boundaries above.
See progress and resumption steps.
