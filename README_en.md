# AWHDL

## Agentic Workflow Harness Description Language

[日本語（正本）](README.md) | English reference translation

> This English document is a reference translation. The [Japanese
> document](README.md) is normative and takes precedence if the versions
> differ.

**AWHDL stands for Agentic Workflow Harness Description Language.** It is a
VHDL-inspired language for describing AI and tool orchestration as
typed, event-driven systems. It describes devices, signals, processes, timing,
budgets, and information-flow constraints without making an LLM the trusted
control plane.

## What AWHDL is for

AWHDL is a language for **designing bounded feedback loops that coordinate
multiple AIs and MCP tools**. A design can connect local or remote language
models, specialized agents, MCP servers, deterministic programs, human
approval, and external services as explicit devices. It then specifies how
data and events move between them, when work may run in parallel, how a result
is judged, and whether the loop retries, asks a human, or completes.

Its underlying goal is to advance **loop engineering**: the engineering
discipline of designing, analyzing, simulating, observing, and improving
iterative AI/tool systems in terms of their topology, state transitions,
acceptance criteria, stopping conditions, resource bounds, and safety
boundaries. AWHDL treats a loop as a reproducible engineered artifact rather
than a prompt sequence that happens to work.

A representative loop is:

```text
task
  -> coding AI
  -> compiler, tests, and proof tool (in parallel through MCP)
  -> review AI
  -> deterministic acceptance judge
  -> retry with diagnostics, request approval, or complete
```

This is deliberately more than a prompt format or a sequential agent script.
AWHDL is intended to make the topology and control rules of an AI system
reviewable before execution:

- which AI or tool is allowed to receive each value;
- which operations are local, remote, deterministic, or human-controlled;
- which events activate a process and which states form a feedback loop;
- how retries, timeouts, token/cost budgets, cancellation, and completion are
  bounded independently of model output;
- where confidential data may flow and when cloud export needs approval;
- what evidence and provenance must be retained for auditing.

The trusted AI Conductor runtime—not an LLM—parses the design and enforces
scheduling, information-flow policy, capabilities, budgets, deadlines,
approval gates, and completion conditions. Models propose content; the
conductor controls execution. This separation is meant to make long-running
multi-AI workflows safer, reproducible, testable, and portable across model
and tool providers.

AWHDL targets workflows such as code generation with compile/test/review
feedback, research with independent retrieval and verification agents,
formal-proof loops, simulation and engineering-tool pipelines, and local-first
workflows that selectively call cloud models. It is inspired by hardware
description languages, but it does not describe electronic circuits and is
not intended to synthesize hardware.

**Harness** means the controlled structure that connects execution components
and binds their data flow, activation conditions, iterations, budgets,
permissions, and safety boundaries. The H in AWHDL therefore does not mean
Hardware. The language borrows useful design concepts from hardware
description languages while describing harnesses for AI/MCP loops.

This repository contains the draft v0.2 specification and its Rust
implementation. `aic check` parses and statically checks the v0.1 profile
syntax, and `aiconductor run-design` binds a design to AI Conductor routes and
executes it in delta cycles. For an introduction, see the Japanese
[AWHDL tutorial (PDF)](docs/tutorial/awhdl_tutorial_ja.pdf).

## Project names

- **AI Conductor**: the complete orchestration system.
- **AWHDL**: Agentic Workflow Harness Description Language, the harness
  description language for AI/MCP loops.
- **cvim**: a thin Vim-oriented frontend.
- **aic**: the provisional command-line executable.

## Quick start

Requirements:

- a Rust toolchain supporting edition 2024;
- Cargo;
- Linux, macOS, or Windows. No model server, API credential, proprietary tool,
  GPU, or machine-specific service is required for Milestone 1.

```bash
cargo test --workspace
cargo run -p conductor-cli -- check examples/hello.awhdl
cargo run -p conductor-cli -- check examples/hello.awhdl --output json
```

Parse and structural errors return a non-zero status and a source location.

## Documentation

| Document | English | 日本語 |
| --- | --- | --- |
| Language specification | [English](docs/AWHDL_LANGUAGE_SPEC_v0.2.md) | [日本語](docs/AWHDL_LANGUAGE_SPEC_v0.2_ja.md) |
| AI Conductor system specification | [English](docs/AI_CONDUCTOR_SYSTEM_SPEC_v0.2.md) | [日本語](docs/AI_CONDUCTOR_SYSTEM_SPEC_v0.2_ja.md) |
| Getting started | [English](docs/GETTING_STARTED.md) | [日本語](docs/GETTING_STARTED_ja.md) |
| Tutorial (TeX / PDF, examples verified on live devices) | — | [PDF](docs/tutorial/awhdl_tutorial_ja.pdf), [TeX](docs/tutorial/awhdl_tutorial_ja.tex) |
| Implementation status | [English](docs/IMPLEMENTATION_STATUS.md) | [日本語](docs/IMPLEMENTATION_STATUS_ja.md) |
| Security model | [English](docs/SECURITY_MODEL.md) | [日本語](docs/SECURITY_MODEL_ja.md) |
| Design decisions | [English](docs/DESIGN_DECISIONS.md) | [日本語](docs/DESIGN_DECISIONS_ja.md) |
| Publication checklist | [English](docs/PUBLICATION_CHECKLIST.md) | [日本語](docs/PUBLICATION_CHECKLIST_ja.md) |
| cvim boundary | [English](cvim/README.md) | [日本語](cvim/README_ja.md) |
| Implementation guide | [English](IMPLEMENTATION_GUIDE_v0.2.md) | [日本語](IMPLEMENTATION_GUIDE_v0.2_ja.md) |
| v0.1 profile (scope and status) | [English](docs/PROFILE_v0.1.md) | [日本語](docs/PROFILE_v0.1_ja.md) |
| Language specification (normative entry point) | [English](docs/LANGUAGE_SPEC.md) | [日本語](docs/LANGUAGE_SPEC_ja.md) |
| Runtime specification | [English](docs/RUNTIME_SPEC.md) | [日本語](docs/RUNTIME_SPEC_ja.md) |
| Security specification | [English](docs/SECURITY_SPEC.md) | [日本語](docs/SECURITY_SPEC_ja.md) |
| IR specification | [English](docs/IR_SPEC.md) | [日本語](docs/IR_SPEC_ja.md) |
| Implementation guide (v0.1) | [English](docs/IMPLEMENTATION_GUIDE.md) | [日本語](docs/IMPLEMENTATION_GUIDE_ja.md) |
| Specification refinement decisions | — | [日本語](docs/REFINEMENT_DECISIONS_ja.md) |

The specifications describe the intended design. The v0.1 scope and each
feature's status are defined only by the matrix in
[PROFILE_v0.1](docs/PROFILE_v0.1.md), which a test keeps consistent with the
code; the implementation-status document summarizes it.

The Japanese documents are normative. The English documents are reference
translations; if the two versions differ, the Japanese version takes
precedence.

## Workspace

- `awhdl-ast`: AST definitions and source byte spans.
- `awhdl-parser`: Pest grammar and AST construction.
- `awhdl-checker`: structural, information-flow and profile checks.
- `conductor-cli`: the `aic` command-line frontend.
- `aiconductor-runtime`: compiles AWHDL designs and runs them with delta
  cycles; Effects, approval, capabilities and completion
  (`aiconductor run-design`).

## Development checks

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Current limitations

Open items are listed under "v0.1 gaps" in
[PROFILE_v0.1](docs/PROFILE_v0.1.md). Runtime tests use the bundled sanitized
configuration in `crates/aiconductor-runtime/tests/fixtures/project` and need no
LLM, MCP server or human; live checks with real devices are the examples under
`examples/`. Unsupported syntax is rejected; it is never silently accepted.

## License

AWHDL is available under the [MIT License](LICENSE).
