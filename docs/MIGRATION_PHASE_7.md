# Phase 7 migration

[日本語（正本）](MIGRATION_PHASE_7_ja.md) | English reference translation

> The Japanese version is canonical. This English version is a reference
> translation; where they differ, the Japanese version prevails.

Phase 7 changes documents and adds one consistency test; runtime behavior,
configuration and the checkpoint format (v5) are unchanged.

## Where things moved

| Topic | Before | Now |
|---|---|---|
| v0.1 scope | milestones in `CODEX_IMPLEMENTATION_INSTRUCTIONS.md`, v0.1 specs, implementation status prose | `docs/PROFILE_v0.1.md` only |
| Feature status | `awhdl/docs/IMPLEMENTATION_STATUS*.md`, progress notes | PROFILE_v0.1 matrix (status documents summarize) |
| Approval, capability | `docs/RUNTIME_SPEC.md` Phases 4–5 | `docs/SECURITY_SPEC.md` |
| Audit rules | scattered in runtime and migration notes | `docs/SECURITY_SPEC.md` |
| Language entry point | root v0.1 / awhdl v0.2 specs | `docs/LANGUAGE_SPEC.md` → AWHDL v0.2 (Japanese normative) |
| Implementation practice | agent briefs | `docs/IMPLEMENTATION_GUIDE.md` (non-normative) |
| v0.2 candidates | — | `docs/PROFILE_v0.2.md` (planning) |

## New obligation

Any change that alters a feature's status updates its PROFILE_v0.1 row in the
same change. `profile_matrix_is_backed_by_existing_tests` fails when a row
claims `yes` without tests, names a nonexistent test, or leaves an untested
`partial` unexplained.

## Verification

From `awhdl/`: `cargo test --workspace --offline`, `cargo fmt --check`, and
`cargo clippy --workspace --all-targets --offline -- -D warnings`.
