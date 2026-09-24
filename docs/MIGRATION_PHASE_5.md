# Phase 5 migration and verification

[日本語（正本）](MIGRATION_PHASE_5_ja.md) | English reference translation

> The Japanese version is canonical. This English version is a reference
> translation; where they differ, the Japanese version prevails.

Phase 5 unifies tool, filesystem, sandbox, network and model permissions into
one parameterized capability model. The authoritative checkpoint is now
[`execution-v5.schema.json`](schema/execution-v5.schema.json). Formats v1–v4
remain historical and are rejected on restore: an existing Effect cannot be
retroactively assigned the capability that authorized it.

## New configuration: `config/capabilities.toml`

Required at load time. Anything not granted is denied. Configuration load fails
if any route's static needs are not granted, if a grant names an unknown route,
or if a grant's class exceeds its route's `effect_class`. When adding a route or
tool, add a grant in the same change.

The shipped grants reproduce the previous behavior, with these deliberate
changes:

- **File arguments are scoped.** Lean, Prolog and MATLAB file tools may read
  only `examples/**`, `.runtime/runs/**`, `.runtime/mcp/**`,
  `.runtime/open-data/**` and `.runtime/full-loop/**`. Previously any existing
  path, including absolute paths and `..`, was accepted by the runtime.
  `.runtime/secrets` and `config/` are not readable through these tools.
- **KDB classes are per tool.** `get_*` and `list_server_limits` are `read` and
  no longer enter the Effect journal; `load_*`, `save_*`, `prune_state` and
  `run_q` stay `local_write`. KDB tools not listed are denied.
- **Prolog** is limited to `run_prolog*`; **MATLAB** to its own server.
- **Codex** routes hold explicit sandbox capabilities. `open_data_acquisition`
  holds `sandbox.full_access` and `network.connect host:*`, recorded as broad
  grants because Codex cannot restrict hosts.
- **Models** are granted per route and profile, including the planner.

`mcp-servers.toml`'s `capability` strings are descriptive only and are not
enforced.

## Limits

- Classification/location-aware authorization and the cloud gateway are not
  implemented.
- Passthrough tool arguments (KDB, filter) are not inspected by the runtime.
- No credential broker; revocation is a policy API and a config edit, not a
  live revocation channel for a running agent.
- Narrowing uses conservative containment and may reject some valid subsets.

## Verification

From `awhdl/`: `cargo test --workspace --offline`, `cargo fmt --check`, and
`cargo clippy --workspace --all-targets --offline -- -D warnings`. The offline
`dataflow_checkpoint` example authorizes through a policy and emits a v5
checkpoint for schema validation. The production configuration is validated by
the config tests. No live provider call is required.
