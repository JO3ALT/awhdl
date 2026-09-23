# AI Conductor System Specification v0.2

[日本語（正本）](AI_CONDUCTOR_SYSTEM_SPEC_v0.2_ja.md) | English reference translation

> This English document is a reference translation. The Japanese document is
> normative and takes precedence if the versions differ.

> **Status:** Public architecture draft. This document defines the target
> system and security boundaries; it is not an operational deployment guide
> and contains no required hostnames, filesystem paths, credentials, or vendor
> accounts. See the repository's implementation-status document for
> implemented behavior.

Examples use placeholder projects, devices, and destinations. Deployments MUST
supply their own policy outside the workflow source, and project-local policy
MUST NOT elevate permissions granted by outer system or user policy.

## 1. Purpose

This document specifies **AI Conductor**, a local-first orchestrator for AI agents, MCP servers, deterministic tools, humans, and cloud services.

Primary goals:

1. Run multi-stage AI/MCP workflows with feedback loops.
2. Advance **loop engineering** by making multi-AI/MCP loops designable,
   analyzable, simulatable, observable, verifiable, and improvable.
3. Keep the human editor, especially Vim, as the main interaction point.
4. Allow local and cloud models to coexist.
5. Prevent designated project data from leaving the local trust boundary.
6. Make external effects explicit and auditable.
7. Keep the orchestration layer independent of any single agent vendor.
8. Support future AWHDL compilation as the preferred workflow-description frontend.

The intended architecture is:

```text
Vim / CLI
    |
    v
Vim UI (`cvim`)
    |
    v
Orchestrator runtime
    |
    +-- Policy engine
    +-- Event scheduler
    +-- State/checkpoint store
    +-- Sandbox manager
    +-- Audit logger
    |
    +----------------------------+
    |                            |
    v                            v
Local execution zone        Cloud gateway
    |                            |
    +-- local LLM                +-- policy re-check
    +-- MCP servers              +-- DLP/content scan
    +-- compiler/test            +-- allowed cloud models
    +-- MATLAB/Lean/Prolog
    +-- local transforms
```

---

## 2. High-level design principle

The LLM must not be the trusted control plane.

The trusted control plane consists of:

- policy engine,
- scheduler/runtime,
- sandbox boundary,
- information-flow checks,
- deterministic completion checks,
- execution budgets,
- human approval gates.

Agents are untrusted or partially trusted computational devices.

---

## 3. Vim integration

The user-facing integration should remain thin.

Typical examples:

```vim
:'<,'>w !cvim review
:'<,'>w !cvim proof
:'<,'>w !cvim refactor
:'<,'>w !cvim test
```

The adapter should:

1. read stdin or files,
2. identify the requested workflow,
3. attach project context allowed by policy,
4. invoke the runtime,
5. return results or a patch to stdout,
6. never implement workflow logic itself.

The preferred architectural role of `cvim` is therefore:

> editor/CLI adapter, not monolithic orchestrator.

---

## 4. Runtime layers

### 4.1 Frontend layer

Inputs:

- AWHDL source
- CLI command
- Vim selection
- project configuration
- optional plain JSON IR for debugging

Outputs:

- workflow IR
- diagnostics
- patches
- results
- approval requests
- audit references

### 4.2 Compiler/policy-analysis layer

Responsibilities:

- parse AWHDL,
- validate types and ports,
- validate information flows,
- validate capabilities,
- validate budgets,
- detect obvious cycles without limits,
- validate cloud-export constraints,
- produce executable IR.

### 4.3 Event runtime

Responsibilities:

- event queue,
- process scheduling,
- timer scheduling,
- asynchronous device calls,
- barriers,
- retries,
- timeouts,
- persistent state,
- checkpoints,
- deterministic signal-update semantics.

### 4.4 Device adapters

Supported device classes:

- `agent`
- `mcp`
- `deterministic`
- `human`
- `declassifier`
- `external_service`

### 4.5 Sandbox manager

Responsibilities:

- create ephemeral execution environments,
- mount only allowed project data,
- apply network policy,
- apply filesystem policy,
- set CPU/RAM/process/time limits,
- destroy sandbox after use,
- collect only declared outputs.

### 4.6 Cloud gateway

The cloud gateway is the only component allowed to transmit workflow payloads to public cloud AI services.

Responsibilities:

- destination allowlist,
- classification check,
- taint check,
- DLP/content scan,
- secret scan,
- path-independent content scan,
- audit logging,
- size/token limits,
- redaction/approved transformation integration.

All general MCP workers should be denied direct internet access unless explicitly required.

---

## 5. Trust zones

### 5.1 Local restricted zone

May process:

- `public`
- `internal`
- `confidential`
- `restricted`
- optionally `secret`

Typical components:

- local LLM
- local MCP servers
- Python
- Rust tools
- MATLAB
- Lean
- Prolog
- compilers
- test runners
- anonymizers

### 5.2 Cloud zone

May process only classifications allowed by the project policy.

Example:

```text
cloud model clearance = internal
```

Then `confidential` and `restricted` data must never be transmitted directly.

### 5.3 Human approval zone

Used for:

- applying patches to the real working tree,
- `git push`,
- email/message sending,
- deployments,
- destructive actions,
- explicit declassification where policy demands review.

---

## 6. Project data policy

A project may contain data with different cloud-export rules.

Example project:

```text
project/
├── src/                 cloud OK
├── docs/                cloud OK
├── data/
│   ├── public/          cloud OK
│   ├── anonymized/      cloud OK
│   └── raw/             LOCAL ONLY
├── secrets/             LOCAL ONLY
└── policy.yaml
```

Example policy:

```yaml
data_policy:
  cloud_allowed:
    - src/**
    - docs/**
    - data/public/**
    - data/anonymized/**

  local_only:
    - data/raw/**
    - secrets/**
    - "**/*.key"
    - "**/*.pem"
```

Path-based rules are necessary but insufficient.

---

## 7. Structured-data policy

Column-level restrictions must be supported.

Example:

```yaml
structured_data:
  patient.csv:
    deny_columns:
      - name
      - address
      - insurance_id
      - birth_date

    allow_columns:
      - age_group
      - sex
      - diagnosis_code
      - cost
```

Cloud-bound processing should use an approved local transformation:

```text
raw dataset
    |
    v
local trusted transform
    |
    v
approved derived dataset
    |
    v
cloud gateway
    |
    v
cloud model
```

---

## 8. Classification and taint

Recommended classes:

```text
public
internal
confidential
restricted
secret
```

Rules:

1. Derived data inherits the strongest classification of its inputs.
2. Copying data to another file path does not change classification.
3. LLM output is tainted by all inputs used to produce it unless a trusted policy says otherwise.
4. Classification can only be lowered by an explicit approved declassifier.

---

## 9. Declassification

A declassifier is a trusted local component.

Examples:

- remove specified columns,
- aggregate by group,
- k-anonymity validation,
- pseudonymization,
- redact identifiers,
- source-code-only extraction,
- generate a patch without local data,
- statistical summarization under defined rules.

Example logical flow:

```text
RESTRICTED
   |
   v
trusted aggregate
   |
   v
k-anonymity check
   |
   v
INTERNAL
   |
   v
cloud model
```

The LLM itself must never be authorized to lower data classification.

---

## 10. Cloud gateway

All public-cloud LLM/API calls should pass through one gateway.

Conceptual path:

```text
Agent
  |
  +-- local-file MCP
  +-- local-MATLAB MCP
  +-- local-Prolog MCP
  +-- local-Lean MCP
  |
  `-- cloud-LLM gateway
          |
          +-- policy engine
          +-- taint/classification check
          +-- DLP scan
          +-- secret scan
          +-- destination check
          |
          v
       Cloud API
```

The gateway should detect:

- secret keys,
- private keys,
- email addresses,
- phone numbers,
- IDs,
- explicitly restricted strings/patterns,
- high-entropy secrets,
- forbidden structured fields,
- copied restricted content,
- payload size violations.

---

## 11. Defense in depth

Cloud egress protection must exist at two levels:

### Level A: source-side policy
The workflow should not expose restricted data to a cloud-capable component.

### Level B: egress-side inspection
The gateway must scan the outgoing payload regardless of its file path or origin.

This protects against cases such as:

```text
data/raw/patient.csv
        |
        v
agent copies content
        |
        v
/tmp/foo.txt
```

A path-only policy would fail. A content-aware egress policy can still deny transmission.

---

## 12. MCP capability model

Each MCP and tool must be assigned capabilities.

Suggested action classes:

```text
SAFE
READ
WRITE
EXTERNAL
DESTRUCTIVE
```

Examples:

| Tool | Class |
|---|---|
| file.read | READ |
| compiler.build | SAFE |
| matlab.run | SAFE |
| file.write in sandbox | WRITE |
| git.commit | WRITE |
| git.push | EXTERNAL |
| email.send | EXTERNAL |
| delete production data | DESTRUCTIVE |

Automatic loop execution should normally permit only:

```text
SAFE
READ
WRITE within sandbox
```

`EXTERNAL` and `DESTRUCTIVE` actions should require policy approval, usually human approval.

---

## 13. Sandbox model

Default execution environment:

- rootless container,
- read-only base filesystem,
- no privileged mode,
- no host PID namespace,
- no host network,
- no Docker socket,
- drop Linux capabilities,
- seccomp,
- AppArmor/SELinux where available,
- CPU limit,
- RAM limit,
- PID/process limit,
- wall-time limit,
- controlled temporary storage.

For higher-risk code:

- gVisor,
- Kata Containers,
- microVM such as Firecracker,
- dedicated VM.

Unknown third-party code should preferentially run in a stronger isolation tier.

---

## 14. Workspace model

Do not let an agent mutate the real repository directly by default.

Preferred flow:

```text
real repository
    |
    +-- git worktree / temporary copy
              |
              v
          sandbox
              |
         agent changes
              |
              v
          git diff
              |
              v
        orchestrator
              |
        human review
              |
              v
        apply to real tree
```

This is particularly compatible with a Vim-centered workflow.

---

## 15. Secret handling

Secrets should not be placed directly in agent-visible environment variables when avoidable.

Preferred model:

```text
Agent
  |
  v
MCP / service adapter
  |
  v
credential broker
  |
  v
external service
```

The agent should receive the capability, not the raw credential.

Never expose:

```text
~/.ssh
~/.aws
general credential stores
Docker socket
host-wide config directories
```

unless a narrowly scoped, reviewed tool requires it.

---

## 16. Network policy

Default:

```text
DENY
```

Allow only destinations required by a particular device.

Example:

```yaml
network:
  default: deny

  devices:
    cloud_llm:
      allow:
        - api.provider-a.example
        - api.provider-b.example

    github_reader:
      allow:
        - github.com
        - api.github.com

    matlab:
      allow: []
```

Where practical, route outbound traffic through an audited proxy.

---

## 17. Execution budget

Every autonomous loop should have a machine-enforced budget.

Recommended limits:

```yaml
budget:
  max_iterations: 10
  max_wall_time: 30m
  max_llm_tokens: 200000
  max_tool_calls: 100
  max_network_requests: 20
  max_disk_write: 500MB
```

Budget exhaustion is an event, not a model decision.

---

## 18. Completion and judging

The agent must not be trusted to decide completion merely by saying "done".

Preferred completion logic:

```text
Coder
  |
  v
candidate
  |
  +-- build
  +-- unit tests
  +-- lint
  +-- proof/formal check
  +-- policy checks
          |
          v
      deterministic judge
          |
      +---+---+
      |       |
     fail    pass
      |       |
    retry   complete
```

Example:

```python
if (
    build.exit_code == 0
    and tests.failed == 0
    and lint.errors == 0
    and proof.ok
):
    complete()
```

LLM-based review may be an additional signal, but deterministic checks should dominate where available.

---

## 19. Event model

The runtime should use an event queue.

Typical event sources:

- signal update,
- MCP completion,
- LLM response,
- file change,
- timer,
- timeout,
- human approval,
- external webhook/event in future,
- budget exhaustion,
- sandbox failure.

AWHDL will describe these relationships; the runtime executes them.

---

## 20. Checkpoints

Persistent state should be checkpointed for:

- long-running workflows,
- human approval waits,
- process restart,
- debugging,
- auditability.

Checkpoint must not implicitly persist secrets or disallowed raw data outside its trust zone.

---

## 21. Audit log

Record:

- workflow ID,
- device invocations,
- timestamp,
- input classification,
- output classification,
- destination,
- policy decision,
- cloud export hash/metadata,
- human approval events,
- budget consumption,
- sandbox lifecycle,
- errors and retries.

Avoid writing restricted payloads to normal logs by default.

Support metadata-only or redacted logs.

---

## 22. Backend independence

The runtime should permit multiple execution backends.

Potential backends:

- native Rust/Tokio runtime,
- LangGraph,
- Microsoft Agent Framework,
- future distributed runtime.

AWHDL is the frontend description language; backend frameworks are implementation targets, not the primary authoring interface.

---

## 23. Proposed internal IR

Use a stable JSON IR.

Example:

```json
{
  "workflow": "secure_development",
  "devices": {},
  "signals": {},
  "processes": [],
  "policies": [],
  "budgets": [],
  "assertions": []
}
```

The IR should be versioned:

```json
{
  "ir_version": "0.1"
}
```

---

## 24. Initial CLI

Proposed commands:

```text
aic check workflow.awhdl
aic compile workflow.awhdl -o workflow.json
aic sim workflow.awhdl
aic run workflow.awhdl
aic graph workflow.awhdl
aic audit <run-id>
```

Editor adapter:

```text
cvim <workflow-name>
cvim run <workflow.awhdl>
cvim review
cvim proof
cvim test
```

---

## 25. Error categories

Suggested diagnostic families:

```text
E1xx syntax/parser
E2xx type/port
E3xx security/data-flow
E4xx capability
E5xx timing/event
E6xx budget/resource
E7xx backend/runtime
E8xx sandbox
E9xx assertion/testbench
```

Example:

```text
AWHDL-E303 CLOUD EXPORT DENIED
signal: patient_data
classification: restricted
destination: cloud_reviewer
clearance: internal
```

---

## 26. MVP architecture

For v0.1:

```text
AWHDL
  |
  v
Parser
  |
  v
AST
  |
  +-- type checker
  +-- security checker
  |
  v
JSON IR
  |
  v
Rust/Tokio runtime
  |
  +-- MCP stdio adapter
  +-- local command adapter
  +-- agent adapter
  +-- timer/event scheduler
  +-- policy engine
  +-- sandbox abstraction
```

Start with a single-host runtime.

Do not begin with Kubernetes or distributed execution.

---

## 27. Recommended repository layout

```text
awhdl/
├── Cargo.toml
├── README.md
├── docs/
│   ├── LANGUAGE_SPEC.md
│   ├── ORCHESTRATOR_SPEC.md
│   └── SECURITY_MODEL.md
├── crates/
│   ├── awhdl-parser/
│   ├── awhdl-ast/
│   ├── awhdl-checker/
│   ├── awhdl-ir/
│   ├── awhdl-runtime/
│   ├── awhdl-mcp/
│   ├── awhdl-policy/
│   ├── awhdl-sandbox/
│   └── awhdl-cli/
├── examples/
│   ├── hello.awhdl
│   ├── code_loop.awhdl
│   └── secure_hybrid.awhdl
└── tests/
    ├── parser/
    ├── security/
    ├── runtime/
    └── integration/
```

---

## 28. Non-goals for v0.1

Do not implement initially:

- graphical editor,
- distributed scheduler,
- Kubernetes operator,
- full formal verification,
- full VHDL compatibility,
- arbitrary embedded scripting,
- production-grade DLP engine,
- vendor-specific rich IDE plugin,
- unrestricted shell agent.

The first goal is a small, testable, secure runtime and language core.

---

## 29. Definition of success for the prototype

A successful v0.1 should demonstrate:

1. Parse a small AWHDL file.
2. Build a typed AST.
3. Reject an invalid signal-to-port connection.
4. Reject a restricted-to-cloud flow.
5. Execute a local MCP call.
6. Execute a timer-triggered process.
7. Execute a retry loop with max iterations.
8. Run two validation tools in parallel.
9. Stop on timeout or budget exhaustion.
10. Produce an audit log.
11. Keep restricted data from being passed to a cloud adapter.
12. Return a patch/result to the Vim/CLI adapter.


---

## 30. Canonical naming

| Name | Role |
|---|---|
| **AI Conductor** | Complete orchestration system |
| **AWHDL** | Agentic Workflow Harness Description Language for AI/MCP loops |
| **cvim** | Vim-oriented frontend |
| **AI Conductor IR** | Checked internal representation |
| **AI Conductor Runtime** | Native event-driven runtime |

`cvim` is not the runtime; AWHDL is not the runtime; MCP is not a security
boundary; an LLM is not the trusted control plane.

The H in AWHDL means Harness, not Hardware. A harness binds device
connectivity, information flow, events, iterations, budgets, approvals, and
safety boundaries into a controlled structure. AWHDL borrows design concepts
from VHDL but its name does not imply hardware description or synthesis.

## 31. Revised architecture

```text
Human -> Vim -> cvim -> AI Conductor
                       ├─ AWHDL parser/compiler
                       ├─ static checker
                       ├─ AI Conductor IR
                       ├─ event runtime
                       ├─ policy engine
                       ├─ sandbox manager
                       ├─ local devices/MCPs
                       ├─ cloud gateway
                       └─ human approval
```

## 32. cvim responsibilities

cvim may submit buffers/selections/files, select workflows, invoke check/sim/run,
display diagnostics and diffs, request approvals, inspect runs, and cancel runs.

cvim must not decide data-export policy, hold unrestricted cloud credentials,
bypass the gateway, or implement trusted sandbox/security logic.

## 33. Project-local configuration

Recommended layout:

```text
project/
├── .conductor/
│   ├── policy.yaml
│   ├── devices.yaml
│   └── workflows/
│       ├── review.awhdl
│       └── develop.awhdl
├── src/
├── docs/
└── data/
```

A project policy may reduce privileges but must not elevate privileges beyond
outer system/user policy.

## 34. Policy precedence

Deny by default. Effective permission is the intersection of system, user,
project, workflow, and device permissions. Any applicable explicit deny wins.

## 35. Two-stage cloud protection

1. Static AWHDL information-flow checking.
2. Runtime gateway checking of actual payload, taint, destination, DLP rules,
   and secrets.

Static success never bypasses runtime egress checks.

## 36. Data provenance

Where practical, runtime values carry classification, source, transform chain,
declassification record, and run ID. The first implementation may track this at
signal/value granularity.

## 37. Cancellation and failure containment

Cancellation stops new scheduling, terminates cancellable calls and sandbox
children, preserves a minimal audit record, and must not apply unapproved
changes to the real repository.

Device failures become structured runtime events rather than crashing the whole
orchestrator where isolation is feasible.

## 38. v0.2 MVP boundary

Required: parser/AST, static classification checking, local/cloud devices, JSON
IR, Tokio event runtime, MCP stdio, timers/timeouts, bounded loops, parallel
validation, audit logging, temporary workspace abstraction, cloud-egress
authorization, and a CLI usable by cvim.

Deferred: full DLP, distributed runtime, Kubernetes, GUI, full temporal logic,
production microVM backend, and rich cvim UI.

## Phase 1 execution identity addendum (2026-09-23)

[Runtime semantics](docs/RUNTIME_SPEC.md) and
[IR specification](docs/IR_SPEC.md) are normative for execution identity
and take precedence within that scope. The integration runtime implements
invocation / generation / correlation, checkpoint restoration and generation-aware
barrier APIs. The AWHDL compiler remains Milestone 1; this does not implement
AWHDL compile/run, signal/event scheduling or CLI resume.
See [migration and implementation limits](docs/MIGRATION_PHASE_1.md).

## Phase 2 Value / Event / Invocation separation (2026-09-23)

Value is latest state, Event is a one-shot occurrence, and Invocation owns call
lifecycle and result. Equal assignments do not emit changed events; distinct
events with equal payloads remain distinct. [Runtime spec](docs/RUNTIME_SPEC.md)
and [IR spec](docs/IR_SPEC.md) are normative and take precedence in this scope.
Format v2 persists consumption to prevent event redelivery, without claiming
exactly-once external effects. New sensitivity syntax and delta-cycle execution
remain unimplemented. See [migration and limits](docs/MIGRATION_PHASE_2.md).

## Phase 3 Effect / Retry / Idempotency (2026-09-23)

Writes enter an Effect journal before dispatch. An ambiguous outcome becomes
`UNCERTAIN` and cannot be resent automatically. The runtime assigns an
idempotency key stable for the same action instance. [Runtime spec](docs/RUNTIME_SPEC.md)
and [IR spec](docs/IR_SPEC.md) are normative and take precedence in this scope.
The current checkpoint format is v3. Trusted reconciliation is a runtime API;
automatic provider lookup and CLI resume are not implemented.
See [migration and limits](docs/MIGRATION_PHASE_3.md).

## Phase 4 Action-bound approval (2026-09-23)

Human approval is not a boolean. It is bound to the SHA-256 hash of one
canonical Effect instance (run, generation, effect, action, capability,
resource, class, content hash), is single-use, expires, and is consumed in the
same checkpoint transition that marks the Effect `STARTED`. External and
destructive writes require approval by default. Only `single_action` scope is
accepted. The current checkpoint format is v4. No approval adapter (HumanPort)
is connected yet, so a required approval fails closed. See
[migration and limits](docs/MIGRATION_PHASE_4.md).

## Phase 5 Capability model (2026-09-23)

Tool, filesystem, sandbox, network and model permissions are one parameterized
capability model (`config/capabilities.toml`), default deny, queried through a
single `authorize(subject, request)`. The authorizing capability's effect class
drives journaling, retry and approval per tool; Effects and approvals bind to
its handle. Planner-supplied file paths are canonicalized and must lie in granted
project paths. The current checkpoint format is v5. Classification-aware export
control and a credential broker are not implemented. See
[migration and limits](docs/MIGRATION_PHASE_5.md).

## Phase 6 Hard / soft completion (2026-09-23)

A planner's completion claim only requests evaluation. The runtime decides
`COMPLETE`, `INCOMPLETE`, `BLOCKED`, `FAILED` or `REQUIRES_REVIEW` from the
state machine, Effect journal and typed adapter output. Hard conditions must be
deterministic; model output and planner claims can only be soft. Workflows with
external or destructive steps must be safety-critical with explicit hard
conditions. The AWHDL `completion when hard/soft` syntax is not parsed yet;
conditions are configuration. See
[migration and limits](docs/MIGRATION_PHASE_6.md).

## Phase 7 v0.1 profile (2026-09-23)

The v0.1 implementation scope and feature status are defined only by
[PROFILE_v0.1](docs/PROFILE_v0.1.md). This specification describes the
target language/system; a construct here that the profile does not list as
v0.1 is unsupported in v0.1. Normative set: LANGUAGE_SPEC, RUNTIME_SPEC,
SECURITY_SPEC, IR_SPEC and PROFILE_v0.1 under `docs/`; on conflict the profile
wins, then those specs.
