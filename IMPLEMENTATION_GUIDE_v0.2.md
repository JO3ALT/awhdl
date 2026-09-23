# AWHDL Implementation Guide v0.2

> **Scope and status source of truth (Phase 7, 2026-09-23):** The v0.1 scope
> and each feature's status are defined only by the matrix in
> [PROFILE_v0.1](docs/PROFILE_v0.1.md). This document summarizes it; the
> matrix wins on any difference. This document defines no scope of its own.
## Project: AI Conductor / AWHDL / cvim

[日本語（正本）](IMPLEMENTATION_GUIDE_v0.2_ja.md) | English reference translation

> This English document is a reference translation. The Japanese document is
> normative and takes precedence if the versions differ.

> **Audience:** contributors implementing the public AWHDL prototype. The guide
> is tool-neutral and does not require a particular coding agent, model,
> provider account, host layout, or proprietary service.

You are implementing a prototype of a VHDL-inspired language and runtime for AI/MCP orchestration.

Read these files first and treat them as the current source of truth:

1. `AWHDL_LANGUAGE_SPEC_v0.2.md`
2. `AI_CONDUCTOR_SYSTEM_SPEC_v0.2.md`

Do not silently redesign the project around an existing orchestration framework. The initial implementation should be a small native Rust prototype. Framework backends may be added later.

---

## 1. Primary objective

Build an MVP that can:

- parse a small AWHDL subset,
- construct a typed AST,
- perform static security-flow checks,
- compile to a versioned JSON IR,
- execute the IR using an asynchronous Rust runtime,
- call MCP tools over stdio,
- run event-driven processes,
- support timers and timeouts,
- execute simple parallel branches,
- enforce iteration/time/tool-call budgets,
- distinguish local and cloud devices,
- reject restricted data flowing to cloud devices,
- write structured audit logs,
- expose a CLI suitable for use from Vim.

The runtime must treat agents and MCP servers as devices, not as the trusted control plane.

The implementation should also provide a foundation for **loop engineering**:
iterative multi-AI/MCP systems should be represented as comparable designs,
with observable traces, evaluation results, and stopping reasons that support
systematic analysis and improvement.

---

## 2. Implementation language and libraries

Use Rust.

Preferred choices:

- async runtime: `tokio`
- CLI: `clap`
- serialization: `serde`, `serde_json`
- parser: choose either `chumsky` or `pest`
- errors: `thiserror`; optionally `miette` for diagnostics
- logging/tracing: `tracing`, `tracing-subscriber`
- temporary workspaces: `tempfile`
- IDs: `uuid`
- time: `std::time` / `tokio::time`

Keep dependencies modest.

Do not add a database in the first iteration unless required by a concrete test.

---

## 3. Repository layout

Create approximately:

```text
awhdl/
├── Cargo.toml
├── README.md
├── docs/
│   ├── AWHDL_LANGUAGE_SPEC_v0.2.md
│   └── AI_CONDUCTOR_SYSTEM_SPEC_v0.2.md
├── crates/
│   ├── awhdl-parser/
│   ├── awhdl-ast/
│   ├── awhdl-checker/
│   ├── awhdl-ir/
│   ├── awhdl-runtime/
│   ├── awhdl-mcp/
│   ├── awhdl-policy/
│   └── awhdl-cli/
├── examples/
│   ├── hello.awhdl
│   ├── secure_flow_ok.awhdl
│   ├── secure_flow_denied.awhdl
│   ├── timer_loop.awhdl
│   └── parallel_validation.awhdl
└── tests/
    ├── parser/
    ├── security/
    ├── runtime/
    └── integration/
```

If a Cargo workspace with fewer crates is simpler at first, start smaller, but keep module boundaries aligned with these responsibilities.

---

## 4. Implement only the v0.1 language subset first

Required syntax:

```text
entity
architecture
device
signal
process
if
case
timer
timeout
parallel
budget
assert
```

Required security classes:

```text
public
internal
restricted
```

Required device locations:

```text
local
cloud
```

Required device kinds:

```text
agent
mcp
deterministic
human
declassifier
```

Required MCP transport initially:

```text
stdio
```

Do not attempt complete VHDL compatibility.

---

## 5. Parser and AST

Design explicit AST types.

At minimum represent:

- entity declaration,
- architecture declaration,
- device declarations,
- device kind,
- device location,
- clearance,
- signals,
- signal data type,
- signal classification,
- processes,
- sensitivity/event triggers,
- calls,
- assignments,
- `if`,
- `case`,
- parallel blocks,
- timers,
- timeouts,
- budgets,
- assertions.

Keep source spans where practical so diagnostics can point to source locations.

Add parser tests before implementing runtime behavior.

---

## 6. Security lattice

Implement the ordering:

```text
public < internal < restricted
```

Implement a reusable type:

```rust
enum Classification {
    Public,
    Internal,
    Restricted,
}
```

and a comparison/check method.

A signal may only flow to a destination whose clearance is equal to or higher than the signal classification.

The checker must reject:

```text
restricted -> internal-clearance device
```

A cloud location alone does not determine clearance; both location and clearance matter.

However, support a project/default policy that can deny specific classifications to all cloud devices.

---

## 7. Taint model

For the prototype:

- a derived value receives the maximum classification of its inputs,
- ordinary agent/MCP output must not automatically lower classification,
- only a `declassifier` device may lower classification,
- declassification should be explicit in the AST/IR,
- log every declassification event.

Do not implement sophisticated information-flow inference initially. Make the rules simple and conservative.

---

## 8. Cloud egress boundary

Design the runtime so that cloud device calls pass through a single policy function/service abstraction.

Conceptually:

```rust
authorize_egress(payload_metadata, destination, classification)
```

The runtime must never allow a cloud adapter to bypass this check.

For v0.1, payload inspection may be minimal, but architecture must allow future:

- secret scanning,
- regex DLP,
- structured-field filtering,
- high-entropy detection,
- custom organization rules.

Add a TODO/API boundary rather than embedding cloud calls throughout the runtime.

---

## 9. Device abstraction

Create a trait approximately like:

```rust
#[async_trait]
pub trait Device {
    async fn invoke(
        &self,
        request: DeviceRequest,
        ctx: &ExecutionContext,
    ) -> Result<DeviceResponse, DeviceError>;
}
```

Implement initial adapters:

1. mock deterministic device,
2. local command/mock agent device,
3. MCP stdio device,
4. mock cloud device that goes through egress authorization.

Do not require real cloud-provider credentials for the test suite.

Use mocks for CI/integration tests.

---

## 10. MCP stdio

Implement a minimal MCP stdio client sufficient for calling a configured tool.

Keep it isolated in `awhdl-mcp`.

Requirements:

- spawn configured child process,
- communicate over stdin/stdout,
- enforce timeout,
- capture exit status,
- prevent unrelated stderr from breaking JSON protocol,
- ensure process cleanup,
- return structured error on malformed response.

If implementing full MCP is too large for the first pass, define the adapter interface and provide a minimal test server fixture. Do not fake success in production code.

---

## 11. Runtime event model

Use `tokio`.

Implement:

- event queue/channel,
- signal store,
- timer events,
- process triggers,
- asynchronous device calls,
- signal updates,
- follow-on event generation.

Prefer deterministic semantics.

A practical prototype may execute ready processes in a deterministic queue order while device calls run concurrently.

Document the exact semantics.

---

## 12. Delta-cycle semantics

Implement a simple cycle model:

1. dequeue event,
2. evaluate all processes made ready by that event,
3. stage signal updates,
4. commit signal updates,
5. enqueue events for changed signals,
6. continue.

Avoid immediate recursive signal propagation.

Add tests showing that source order does not change simple signal-chain behavior.

---

## 13. Parallel blocks

Implement a basic parallel block using `tokio::join!` or `JoinSet`.

Example conceptual behavior:

```text
compiler(candidate)
tester(candidate)
lean(candidate)
```

run concurrently and all results are collected before the block completes.

A failure in one branch must produce a structured result and must not leak orphan processes.

---

## 14. Timer and timeout

Support:

```text
timer monitor_clock : period 10 sec;
```

and operation/process timeout.

Tests must use short millisecond-scale timers even if examples use seconds/minutes.

Use paused Tokio time where practical for deterministic tests.

---

## 15. Budgets

Implement at least:

- max iterations,
- max wall time,
- max tool/device calls.

Optional if easy:

- token budget placeholder,
- network request count.

Budget limits must be enforced by the runtime, not by the agent.

Budget exhaustion should emit a structured event/error and be audit logged.

---

## 16. Assertions

Implement a minimal assertion system.

Required initial forms may be simplified to:

- boolean assertions over known runtime/static properties,
- security-flow assertions,
- retry/iteration bound assertions.

If full temporal assertions are too large, parse but do not execute them yet. Emit a clear "not implemented" diagnostic rather than ignoring them.

---

## 17. Sandbox abstraction

Create a sandbox trait/interface now, even if the first backend is a minimal local temporary-directory implementation.

Target interface should allow future backends:

- rootless Docker/Podman,
- gVisor,
- Kata,
- Firecracker/microVM.

Never bind-mount the user's entire home directory by default.

Never expose:

```text
/var/run/docker.sock
~/.ssh
~/.aws
```

in the default sandbox implementation.

The prototype sandbox should operate on a temporary workspace or git worktree.

---

## 18. Workspace behavior

Default development workflow:

```text
real repo
  -> temporary worktree/copy
  -> device modifies sandbox workspace
  -> runtime collects diff/result
  -> user reviews
  -> separate apply step
```

Do not auto-apply generated patches to the real repository in the first version.

---

## 19. CLI

Implement:

```text
aic check <file>
aic compile <file> -o <ir.json>
aic run <file>
aic sim <file>
```

Optional:

```text
aic graph <file>
```

Return non-zero exit codes for:

- parse failure,
- static type failure,
- security-flow violation,
- runtime failure,
- assertion failure.

Diagnostics should be concise and script-friendly.

---

## 20. Vim adapter compatibility

Ensure:

```bash
cat selection.txt | aic run workflow.awhdl
```

or a thin `cvim` wrapper can work.

Output modes should eventually include:

```text
human-readable
json
patch/diff
```

For the first pass, provide:

```text
--output text
--output json
```

---

## 21. Audit logging

Emit JSON Lines (`.jsonl`) for each run.

Record at least:

- run ID,
- timestamp,
- event type,
- device ID,
- device kind,
- location,
- input classification,
- output classification,
- policy allow/deny decision,
- tool/device call count,
- budget state,
- errors,
- declassification event.

Do not include raw restricted payloads in the audit log by default.

---

## 22. Examples to implement

### hello.awhdl

One local device transforms a text input.

### secure_flow_ok.awhdl

`internal` data may flow to an `internal`-clearance cloud mock device if project policy allows it.

### secure_flow_denied.awhdl

`restricted` data connected to an `internal`-clearance cloud device must fail during `aic check`.

Expected result:

```text
AWHDL-E303 CLOUD EXPORT DENIED
```

or equivalent.

### timer_loop.awhdl

A timer activates a process repeatedly but is stopped by an execution budget.

### parallel_validation.awhdl

A candidate goes to multiple mock validation devices concurrently; a deterministic judge decides success.

---

## 23. Tests required before moving on

Add automated tests for:

1. minimal program parses,
2. syntax error reports a useful location,
3. valid security flow passes,
4. restricted-to-cloud flow fails,
5. classification propagates conservatively,
6. declassifier can explicitly lower a classification,
7. timer triggers a process,
8. timeout cancels/fails a device call,
9. parallel devices actually overlap in time,
10. max iterations stops a loop,
11. max tool calls stops execution,
12. audit log is emitted,
13. no restricted payload is written into normal audit log,
14. mock cloud adapter cannot bypass egress authorization,
15. temporary workspace is removed after execution.

Run:

```bash
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

before declaring a milestone complete.

---

## 24. Security rules for the implementation itself

Treat all model/device output as untrusted.

Do not:

- use `shell=true` style command concatenation,
- interpolate untrusted strings into shell commands,
- expose arbitrary host paths without policy validation,
- trust a device-provided classification downgrade,
- allow cloud adapters to invoke raw HTTP independently of the egress layer,
- auto-apply changes to the user's real working tree,
- log secrets or restricted payloads by default.

Use argument arrays and explicit executable paths for subprocesses.

---

## 25. Work style

Proceed in small vertical slices.

Suggested sequence:

### Milestone 1
- workspace skeleton
- AST
- parser
- `aic check`
- basic diagnostics

### Milestone 2
- classification lattice
- signal/port checks
- location/clearance checks
- security tests

### Milestone 3
- JSON IR
- event runtime
- mock device
- one process + one signal chain

### Milestone 4
- timers
- timeouts
- budgets
- parallel execution

### Milestone 5
- MCP stdio adapter
- mock MCP test fixture
- audit logs

### Milestone 6
- sandbox abstraction
- temporary workspace
- mock cloud adapter + egress gateway
- denied-cloud-flow integration test

At the end of each milestone:

1. run tests,
2. run fmt/clippy,
3. update README,
4. summarize changed files,
5. state remaining limitations,
6. do not start the next milestone if current tests fail.

---

## 26. Design questions that must be documented, not guessed away

When an ambiguity appears, prefer a conservative implementation and document it in `docs/DESIGN_DECISIONS.md`.

Important questions include:

- exact delta-cycle scheduling,
- how multiple process writes to one signal are resolved,
- whether cloud location implies additional default restrictions,
- how declassification is authorized,
- how persistent signal checkpoints are encrypted/stored,
- how tool schemas map to AWHDL types,
- how async device errors propagate,
- how cancellation is handled,
- how process sensitivity interacts with explicit events,
- what constitutes a deterministic completion condition.

Do not hide these choices inside implementation details.

---

## 27. First task

Start by creating the Cargo workspace and implementing Milestone 1 only.

Deliver:

- repository skeleton,
- language docs copied into `docs/`,
- AST types,
- parser for the minimal hello example,
- `aic check`,
- parser unit tests,
- README with build/test instructions.

Do not implement cloud APIs, real LLM calls, Docker, or distributed execution in the first task.

After Milestone 1 is complete, report:

- files created,
- grammar implemented,
- tests passing,
- unsupported syntax,
- proposed next step.


---

## 28. Canonical naming — do not rename

```text
AI Conductor   whole system
AWHDL          Agentic Workflow Harness Description Language
cvim           Vim-oriented frontend
aic            provisional command-line executable
```

Dependency direction:

The H means Harness, not Hardware. In the implementation, the harness is the
checked and executable boundary that combines the device graph with event
scheduling, information flow, budgets, approvals, and completion rules.

```text
cvim
  -> AI Conductor CLI/API
       -> AWHDL compiler/checker
       -> runtime
       -> policy
       -> sandbox
       -> MCP adapters
       -> cloud gateway
```

## 29. v0.2 repository target

```text
ai-conductor/
├── Cargo.toml
├── README.md
├── docs/
│   ├── AWHDL_LANGUAGE_SPEC_v0.2.md
│   ├── AI_CONDUCTOR_SYSTEM_SPEC_v0.2.md
│   └── DESIGN_DECISIONS.md
├── crates/
│   ├── awhdl-parser/
│   ├── awhdl-ast/
│   ├── awhdl-checker/
│   ├── conductor-ir/
│   ├── conductor-runtime/
│   ├── conductor-mcp/
│   ├── conductor-policy/
│   ├── conductor-sandbox/
│   └── conductor-cli/
├── cvim/
│   └── README.md
├── examples/
└── tests/
```

For Milestone 1, cvim may remain a design placeholder.

## 30. Security invariant

A repository must never be able to grant itself privileges. Project-local
policy may only reduce effective permissions relative to outer system/user
policy.

## 31. First task

Implement Milestone 1 only:

- Cargo workspace;
- AST;
- parser;
- `aic check`;
- source diagnostics;
- parser tests;
- v0.2 docs in `docs/`;
- `cvim/README.md` describing the thin frontend boundary.

Do not implement real cloud APIs, Docker, a full Vim plugin, or distributed
execution yet.
