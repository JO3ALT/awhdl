# AWHDL Language Specification v0.2
## Agentic Workflow Harness Description Language

[日本語（正本）](AWHDL_LANGUAGE_SPEC_v0.2_ja.md) | English reference translation

> This English document is a reference translation. The Japanese document is
> normative and takes precedence if the versions differ.

> **Status:** Public draft, not a final standard. This document describes the
> intended language through v0.2; it is not a claim that every construct is
> implemented. See the repository's implementation-status document for current
> conformance.
> Product, provider, host, and device names in examples are illustrative and
> non-normative.

The key words **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY**
express normative requirements when written in uppercase. Other explanatory
text and all examples are informative unless stated otherwise.

### 1. Purpose

AWHDL stands for **Agentic Workflow Harness Description Language**. It is the workflow-description language of AI Conductor and is a domain-specific language for describing AI agents, MCP servers, humans, local programs, cloud services, timers, data flows, feedback loops, security classifications, and execution constraints.

Here, *harness* means the controlled structure that connects execution
components and binds their data flow, activation conditions, iterations,
budgets, permissions, and safety boundaries. The H does not mean Hardware.
AWHDL borrows useful ideas such as concurrency, signals, events, and structural
checking from hardware description languages, but its subject is an agentic
workflow involving AIs, MCPs, tools, and humans.

The central design principle is:

> Describe the structure, connectivity, state transitions, event reactions, timing, and information-flow constraints of an agentic system, rather than writing a conventional sequential program.

AWHDL is intended for:

- AI-agent iteration loops
- Multi-MCP coordination
- Local/cloud LLM hybrid workflows
- Human-in-the-loop approval
- Periodic and event-driven execution
- Parallel execution
- Retry and timeout control
- Data-classification enforcement
- Cloud data-loss prevention
- Execution budgets
- Static workflow validation
- Workflow simulation
- Backend synthesis to a runtime or orchestration framework

Its underlying goal is to advance **loop engineering**: treating iterative
multi-AI and multi-MCP systems as engineered artifacts whose topology, state
transitions, evaluation criteria, stopping conditions, resource bounds, and
safety boundaries can be designed, analyzed, simulated, observed, verified,
and improved.

---

## 2. Core language model

The initial language consists of these principal constructs:

```text
entity
architecture
device
port
signal
process
timer
policy
budget
assert
configuration
```

Conceptual mapping to VHDL:

| AWHDL | VHDL analogue | Meaning |
|---|---|---|
| entity | entity | Workflow external interface |
| architecture | architecture | Workflow implementation |
| device | component/entity instance | Agent, MCP, local program, service, or human |
| port | port | Typed device input/output |
| signal | signal | Data/event connection |
| process | process | Event-driven behavior |
| timer | clock/timing source | Time-driven events |
| policy | constraint | Security and execution rules |
| assertion | assertion | Safety/completion invariant |
| configuration | configuration | Backend/device binding |

---

## 3. Minimal example

```vhdl
entity hello is
    port (
        task   : in  text<internal>;
        result : out text<internal>
    );
end hello;

architecture behavioral of hello is

    device llm : local_llm;

begin

    process(task)
    begin
        llm.run(task) -> result;
    end process;

end behavioral;
```

---

## 4. Entity

An `entity` declares the workflow interface.

```vhdl
entity code_review is
    port (
        source : in  code<internal>;
        report : out text<internal>
    );
end code_review;
```

An entity contains no implementation. Multiple architectures may implement the same entity.

---

## 5. Architecture

```vhdl
architecture secure of code_review is

    device reviewer : cloud_reviewer;
    device checker  : local_linter;

begin
    ...
end secure;
```

Typical alternatives:

```text
architecture local
architecture cloud
architecture hybrid
```

This allows the same logical workflow to be deployed with different execution environments.

---

## 6. Device

A `device` represents an execution component.

Examples include:

- LLM agent
- MCP server
- deterministic program
- browser or external service
- human operator
- declassifier
- judge/validator

Example MCP device:

```vhdl
device matlab : mcp
    generic (
        server    => "matlab-mcp",
        location  => local,
        clearance => restricted
    );
```

Example cloud agent:

```vhdl
device cloud_reviewer : agent
    generic (
        provider   => example_cloud,
        model      => "example-review-model",
        location   => cloud,
        clearance  => internal
    );
```

Example local LLM:

```vhdl
device local_agent : agent
    generic (
        provider   => local,
        endpoint   => "http://local-agent.example",
        clearance  => restricted
    );
```

---

## 7. Ports

Devices expose typed ports.

```vhdl
device analyzer : agent is
    port (
        request  : in  text<internal>;
        context  : in  data<internal>;
        response : out text<internal>;
        done     : out event;
        error    : out event
    );
end device;
```

Ports carry both a data type and an information-classification label.

---

## 8. Signals

Signals connect devices and processes.

```vhdl
signal source      : code<internal>;
signal candidate   : code<internal>;
signal test_result : test_result<internal>;
```

Signals are updated using VHDL-like assignment semantics.

---

## 9. Security classification

Built-in classes:

```text
public
internal
confidential
restricted
secret
```

Ordering:

```text
public < internal < confidential < restricted < secret
```

Examples:

```vhdl
signal README       : text<public>;
signal source       : code<internal>;
signal student_data : table<restricted>;
signal api_key      : secret<secret>;
```

---

## 10. Information-flow rule

A value may flow to a device only if the destination is permitted to handle that classification.

Conceptually:

```text
signal.classification <= device.clearance
```

Example:

```vhdl
signal patient : table<restricted>;

device cloud_reviewer : agent
    generic (
        location  => cloud,
        clearance => internal
    );
```

This must be rejected:

```vhdl
cloud_reviewer.input <= patient;
```

Suggested diagnostic:

```text
AWHDL-E203 SECURITY FLOW ERROR

restricted signal "patient"
cannot connect to internal port "cloud_reviewer.input"
```

---

## 11. Taint propagation

Derived data inherits the strongest classification of its inputs unless an explicit trusted declassification operation is used.

```text
A : restricted
B := summarize(A)

=> B : restricted
```

An LLM cannot lower a classification by declaring its own output safe.

---

## 12. Declassification

Only a trusted `declassifier` may lower a classification.

```vhdl
device anonymizer : declassifier
    generic (
        from   => restricted,
        to     => internal,
        method => k_anonymity,
        k      => 5
    );
```

Usage:

```vhdl
signal raw       : table<restricted>;
signal anonymous : table<internal>;

anonymous <= declassify raw
    using anonymizer;
```

---

## 13. Process

A process executes when a signal or event in its sensitivity list changes.

```vhdl
process(task)
begin
    coder.run(task) -> candidate;
end process;
```

Multiple triggers:

```vhdl
process(task, feedback)
begin
    coder.run(task, feedback) -> candidate;
end process;
```

---

## 14. Standard events

Initial standard events:

```text
started
done
failed
changed
timeout
approved
rejected
ready
exhausted
```

Example:

```vhdl
process(test.done)
begin
    if test.result.pass then
        state <= COMPLETE;
    end if;
end process;
```

---

## 15. Time

Time is a first-class type.

Initial units:

```text
ms
sec
min
hour
day
```

Example:

```vhdl
wait for 10 sec;
```

---

## 16. Timer

Periodic execution:

```vhdl
timer monitor_clock : period 10 min;

process(monitor_clock)
begin
    monitor.check;
end process;
```

This is the natural representation of timer-based loops such as a periodic status check.

---

## 17. Timeout

Operation-level timeout:

```vhdl
test.run(candidate)
    timeout 5 min
    -> result;
```

Process-level timeout:

```vhdl
process(candidate)

    timeout 10 min;

begin
    ...

on timeout
    state <= FAILED;

end process;
```

---

## 18. Asynchronous device calls

Device calls are asynchronous by default.

```vhdl
coder.run(task) -> candidate;
```

Semantics:

1. request is emitted,
2. the process need not block,
3. result arrival updates a signal,
4. the signal update triggers dependent processes.

Explicit synchronization:

```vhdl
await coder.run(task) -> candidate;
```

---

## 19. Parallel execution

```vhdl
parallel

    compiler.run(candidate) -> build_result;
    tester.run(candidate)   -> test_result;
    lean.run(candidate)     -> proof_result;

end parallel;
```

---

## 20. Barrier

A barrier emits `ready` when all required signals have produced results.

```vhdl
barrier validation (
    build_result,
    test_result,
    proof_result
);

process(validation.ready)
begin
    ...
end process;
```

---

## 21. FSM support

```vhdl
type workflow_state is (
    IDLE,
    CODING,
    TESTING,
    REVIEWING,
    WAIT_APPROVAL,
    COMPLETE,
    FAILED
);

signal state : workflow_state := IDLE;
```

State transition:

```vhdl
process(test.done)
begin
    case state is
        when TESTING =>
            if test.result.pass then
                state <= REVIEWING;
            else
                state <= CODING;
            end if;

        when others =>
            null;
    end case;
end process;
```

---

## 22. Feedback loop

```vhdl
process(task)
begin
    state <= CODING;
    coder.run(task) -> candidate;
end process;

process(candidate)
begin
    state <= TESTING;
    tester.run(candidate) -> test_result;
end process;

process(test_result)
begin
    if test_result.pass then
        state <= COMPLETE;
    else
        retry_count <= retry_count + 1;

        coder.run(
            task,
            candidate,
            test_result.errors
        ) -> candidate;
    end if;
end process;
```

---

## 23. Execution budget

AI loops should be bounded independently of LLM behavior.

```vhdl
budget development_loop is
    iterations       <= 10;
    wall_time        <= 30 min;
    llm_tokens       <= 200000;
    tool_calls       <= 100;
    network_requests <= 20;
end budget;
```

Budget exhaustion emits:

```text
development_loop.exhausted
```

---

## 24. Retry shorthand

For simple cases:

```vhdl
retry test.run(candidate)
    until result.pass
    max 5
    delay 10 sec;
```

Complex loops should use explicit process/FSM logic.

---

## 25. Human-in-the-loop

Humans are devices.

```vhdl
device operator : human;
```

Approval:

```vhdl
operator.approve(diff) -> approval;
```

Convenience syntax:

```vhdl
approve operator (
    message => "Apply this patch?",
    content => diff
) -> approval;
```

---

## 26. Approval policy

```vhdl
policy external_write is

    require human_approval
        when action.class = external_write;

end policy;
```

Typical external-write actions:

```text
git push
email send
Slack send
database mutation
deployment
destructive operation
```

---

## 27. Location

Built-in location classes:

```text
local
sandbox
private_cloud
cloud
external
```

Example:

```vhdl
device matlab : mcp
    generic (
        location => local
    );
```

---

## 28. Network policy

```vhdl
policy network is

    default deny;

    allow cloud_reviewer
        to "api.example.ai";

    allow github
        to "github.com";

    deny matlab to network;
    deny lean   to network;

end policy;
```

---

## 29. Filesystem policy

```vhdl
policy filesystem is

    allow coder read_write "/workspace";

    deny coder read "~/.ssh";
    deny coder read "~/.aws";
    deny all   read "/etc";

end policy;
```

---

## 30. Cloud export

Cloud-bound data flow should be auditable.

```vhdl
export anonymous
    to cloud_reviewer;
```

`export` is a security-significant operation even if normal signal connections are also supported.

---

## 31. Assertions

Examples:

```vhdl
assert never (
    restricted -> cloud
);
```

```vhdl
assert never (
    external_write and
    not human.approved
);
```

```vhdl
assert always (
    retry_count <= 10
);
```

---

## 32. Temporal assertions

```vhdl
assert eventually (
    task.started -> task.completed
)
within 30 min;
```

Future versions may support an LTL/PSL-compatible subset.

---

## 33. Testbench

```vhdl
entity testbench is
end;

architecture simulation of testbench is

    signal task : text<internal>;

begin

    task <= "Refactor module";

    wait for 1 sec;

    assert workflow.state /= FAILED;

end simulation;
```

---

## 34. Fault injection

```vhdl
inject coder.timeout
    probability 0.05;

inject cloud_reviewer.rate_limit
    probability 0.10;

inject tester.failure
    after 5 min;
```

---

## 35. Security simulation

```vhdl
inject malicious_prompt
    into source;

assert never (
    secret_data -> cloud
);
```

This is intended for prompt-injection and exfiltration testing.

---

## 36. MCP device definition

```vhdl
device matlab : mcp is

    generic (
        transport => stdio,
        command   => "matlab-mcp"
    );

    capability (
        "evaluate",
        "plot",
        "workspace"
    );

    clearance restricted;

end device;
```

Initial MCP transports:

```text
stdio
HTTP
```

---

## 37. Tool-level capability policy

```vhdl
capability matlab.evaluate
    filesystem => workspace,
    network    => deny;

capability github.read
    class => read;

capability github.push
    class => external_write;
```

Suggested action classes:

```text
safe
read
write
external_write
destructive
```

---

## 38. Agent

```vhdl
device coder : agent is

    generic (
        model       => "local-model",
        temperature => 0.2
    );

    tools (
        compiler,
        tester,
        filesystem
    );

end device;
```

Agents receive only explicitly granted tools.

---

## 39. Deterministic judge

Completion should prefer deterministic checks.

```vhdl
device judge : deterministic is

    expression =>
        build.ok
        and tests.failed = 0
        and lint.errors = 0;

end device;
```

---

## 40. Completion condition

```vhdl
completion when
    build.ok
    and test.failed = 0
    and review.score >= 0.9;
```

A model-generated string such as `"COMPLETE"` must not be treated as sufficient proof of completion.

---

## 41. Configuration

Device implementations can be rebound without changing workflow logic.

```vhdl
configuration local_config of code_loop is

    for secure

        for coder : agent
            use entity local.local_agent;
        end for;

    end for;

end local_config;
```

Alternative:

```vhdl
configuration cloud_config of code_loop is

    for secure

        for coder : agent
            use entity cloud.cloud_reviewer;
        end for;

    end for;

end cloud_config;
```

---

## 42. Example hybrid workflow

```vhdl
entity secure_development is

    port (
        task   : in  text<internal>;
        source : in  code<confidential>;
        result : out code<confidential>
    );

end secure_development;

architecture hybrid of secure_development is

    device local_coder : agent
        generic (
            location  => local,
            clearance => restricted
        );

    device cloud_reviewer : agent
        generic (
            location  => cloud,
            clearance => internal
        );

    device compiler : mcp
        generic (
            location => sandbox
        );

    device tester : mcp
        generic (
            location => sandbox
        );

    device sanitizer : declassifier
        generic (
            from => confidential,
            to   => internal
        );

    signal candidate : code<confidential>;
    signal safe_diff : text<internal>;
    signal build     : build_result<internal>;
    signal tests     : test_result<internal>;
    signal review    : review_result<internal>;

    signal retries : integer := 0;

begin

    process(task)
    begin
        local_coder.run(task, source)
            -> candidate;
    end process;

    process(candidate)
    begin
        parallel
            compiler.run(candidate)
                -> build;

            tester.run(candidate)
                -> tests;

            sanitizer.run(candidate)
                -> safe_diff;
        end parallel;
    end process;

    process(safe_diff)
    begin
        cloud_reviewer.run(safe_diff)
            -> review;
    end process;

    process(build, tests, review)
    begin
        if build.ok
           and tests.ok
           and review.approved
        then
            result <= candidate;

        elsif retries < 10 then
            retries <= retries + 1;

            local_coder.run(
                task,
                candidate,
                build.errors,
                tests.errors,
                review.comments
            ) -> candidate;

        else
            raise workflow_failed;
        end if;
    end process;

    assert never (
        source -> cloud_reviewer
    );

    assert retries <= 10;

end hybrid;
```

---

## 43. Compiler pipeline

```text
AWHDL source
      |
      v
Lexer / Parser
      |
      v
AST
      |
      +-- type check
      +-- port check
      +-- security-flow analysis
      +-- capability analysis
      +-- cycle analysis
      +-- budget analysis
      +-- assertion validation
      |
      v
AWHDL IR
      |
      +-- simulator
      +-- local runtime
      +-- LangGraph backend
      +-- Agent Framework backend
      +-- distributed runtime
```

---

## 44. Intermediate representation

The IR may be JSON. Humans should not be required to author it.

Example source:

```vhdl
process(test.done)
begin
    if test.ok then
        state <= COMPLETE;
    else
        state <= CODING;
    end if;
end process;
```

Possible IR:

```json
{
  "process": {
    "trigger": "test.done",
    "actions": [
      {
        "if": "test.ok",
        "then": {
          "assign": ["state", "COMPLETE"]
        },
        "else": {
          "assign": ["state", "CODING"]
        }
      }
    ]
  }
}
```

MCP communication remains JSON/JSON-RPC.

---

## 45. Runtime event model

```text
Event Queue
     |
     v
Scheduler
     |
     +-- process A
     +-- process B
     +-- process C
     +-- timers
             |
             v
          Devices
```

Processes are logically concurrent.

---

## 46. Delta-cycle semantics

Signal update behavior should be deterministic and independent of source ordering.

Initial proposed cycle:

```text
event cycle
   |
   v
process evaluation
   |
   v
signal update
   |
   v
new event detection
   |
   v
next delta cycle
```

This mirrors the useful part of VHDL simulation semantics.

---

## 47. Persistent state

```vhdl
signal context : context_type
    persistent;
```

Persistent signals are checkpointed by the runtime. Ordinary signals are volatile.

---

## 48. Design principles

1. The LLM is a device, not the trusted control plane.
2. Security is enforced by types, policies, sandboxing, and runtime controls, not prompts.
3. Time is a first-class language concept.
4. Agents and MCPs are assumed to operate asynchronously and concurrently.
5. External side effects are explicit.
6. Human approval is a first-class event.
7. Loops are bounded by budgets.
8. JSON is used for protocols and IR, not as the primary human workflow language.
9. Workflows should be simulatable before execution.
10. Workflow description is independent of execution backend.

---

## 49. Minimum viable language v0.1

Implement first:

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
security class
location
budget
assert
```

Security classes:

```text
public
internal
restricted
```

Locations:

```text
local
cloud
```

Transports:

```text
stdio
HTTP
```

---

## 50. Suggested implementation

Rust is recommended for the initial compiler/runtime.

```text
AWHDL
  |
  v
pest / chumsky parser
  |
  v
Rust AST
  |
  v
type + security checker
  |
  v
IR
  |
  v
Tokio event runtime
  |
  v
MCP JSON-RPC
```

Tokio maps naturally to:

```text
process
signal
timer
parallel
event
```

---

## 51. Roadmap

### v0.1
- Parser
- AST
- entity/architecture/device/signal/process
- timer/timeout
- basic security labels
- local/cloud location
- MCP stdio
- event runtime
- basic assertions
- basic execution budgets

### v0.2
- human approval
- declassifier
- persistent signals
- sandbox policy
- network policy
- filesystem policy

### v0.3
- testbench
- fault injection
- security simulation
- temporal assertions

### v1.0
- distributed execution
- formal verification
- workflow synthesis
- multiple execution backends
- graphical circuit/workflow representation

---

## 52. Long-term positioning

AWHDL is not primarily a prompt language. As its formal name states, it is an
**Agentic Workflow Harness Description Language**, and belongs to the broader
category of agentic system description languages.

It is an **Agentic System Description Language** for describing:

- which components exist,
- how they are connected,
- which events activate them,
- how feedback loops behave,
- which timing constraints apply,
- what data may flow to which destination,
- which actions require approval,
- and what conditions prove completion or safety.

The long-term toolchain target is:

```text
              AWHDL source
                   |
          +--------+--------+
          |                 |
          v                 v
      Simulator          Compiler
          |                 |
     testbench         policy check
          |                 |
          +--------+--------+
                   |
                   v
                Runtime
                   |
      +------------+-------------+
      |            |             |
      v            v             v
   Local LLM    MCP Server    Cloud Agent
      |            |             |
      +------------+-------------+
                   |
                   v
                 Human
```


---

## 53. AI Conductor integration

Canonical project structure:

```text
AI Conductor
├── AWHDL              workflow/system description language
├── compiler/checker   static validation and IR generation
├── runtime            event-driven execution engine
├── policy             information-flow/capability enforcement
├── sandbox            isolated execution
├── gateway            controlled cloud egress
└── cvim               Vim-oriented user interface
```

`cvim` is a thin frontend. Trusted policy logic belongs in AI Conductor.

## 54. Source and CLI convention

Recommended extension: `.awhdl`.

Provisional AI Conductor CLI:

```text
aic check workflow.awhdl
aic compile workflow.awhdl
aic sim workflow.awhdl
aic run workflow.awhdl
```

## 55. Timing semantics clarification

AWHDL distinguishes:

1. event-driven execution;
2. timer-driven execution;
3. timeout/deadline constraints.

A timer is orchestration time, not a hardware clock frequency. Feedback loops
should normally be completion-event driven.

## 56. Security labels as types

A signal is conceptually `BaseType<Classification>`, for example
`table<restricted>`. Ordinary transformations conservatively retain the
strongest input classification. Only an explicit trusted declassifier may lower
it. Runtime cloud-egress inspection remains mandatory even after static checks.

## 57. Synthesis

In AWHDL, synthesis means:

```text
AWHDL -> AST -> static checks -> AI Conductor IR -> execution plan -> runtime
```

It does not mean hardware synthesis.

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
