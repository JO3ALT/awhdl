# AWHDL implementation status

[日本語（正本）](IMPLEMENTATION_STATUS_ja.md) | English reference translation

> The Japanese document is normative and takes precedence if the versions differ.

## Release state

- Specification: draft v0.2.
- Compiler prototype: Milestone 1.
- CLI: `aic check` only.
- Runtime conformance: not yet implemented.

## Implemented syntax

- entity declarations and ports;
- architecture declarations;
- simple device declarations;
- simple signal declarations with optional initial values;
- processes with sensitivity lists;
- device calls with a single output;
- signal assignments;
- `null` statements;
- comments beginning with `--`;
- data types with an optional classification spelling, such as
  `text<internal>`.

## Implemented checks

- parse errors with line and column;
- duplicate entities and ports;
- architecture references to unknown entities;
- duplicate devices, signals, and port/signal names;
- unknown process sensitivity names;
- calls to unknown devices;
- calls or assignments to unknown output targets.

## Parsed but not semantically enforced

Classification names are retained in the AST but are not yet validated as a
lattice. A successful check therefore makes no security-flow claim.

## Not implemented

- `if`, `case`, timers, timeout, parallel, budget, and assertions;
- device generics, ports, capabilities, locations, and clearance;
- classification flow, taint propagation, and declassification;
- versioned JSON IR;
- event scheduling and delta cycles;
- MCP or agent invocation from an AWHDL design;
- cloud-egress authorization and sandbox execution;
- `aic compile`, `sim`, `run`, `graph`, and `audit`.

Unsupported syntax must produce a diagnostic rather than being ignored.

## Conformance policy

The specification defines the intended language. Only features listed under
“Implemented syntax” and “Implemented checks” are currently supported by the
prototype. Future milestones should update this document in the same change as
their implementation and tests.
