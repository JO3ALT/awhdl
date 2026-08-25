# AWHDL design decisions

[日本語（正本）](DESIGN_DECISIONS_ja.md) | English reference translation

> The Japanese document is normative and takes precedence if the versions differ.

## Formal name

AWHDL formally stands for **Agentic Workflow Harness Description Language**.
The H means Harness, not Hardware. Harness expresses the design intent of
connecting multiple AIs, MCPs, deterministic tools, and humans while binding
their data flow, events, iterations, budgets, permissions, completion rules,
and safety boundaries into one checkable and executable structure.

VHDL inspired concurrency, signals, processes, events, and structural checks,
but AWHDL does not describe electronic circuits and does not target hardware
synthesis.

## Milestone 1 scope

Milestone 1 implements only the syntax needed for the canonical `hello.awhdl`
vertical slice: entities and ports, architectures, simple device and signal
declarations, processes, device calls, signal assignments, and `null`.

The AST already keeps data-classification spelling and source byte spans so the
Milestone 2 checker can add the classification lattice without replacing the
parser boundary.

## Parsing and diagnostics

The parser uses Pest and accepts lowercase AWHDL keywords. AWHDL is not treated
as fully VHDL-compatible. Syntax outside the implemented subset is rejected as
`AWHDL-E101`; it is never silently ignored.

The checker performs name and structural checks only. Its `E2xx` diagnostics do
not imply that port typing or security-flow analysis has been implemented.

## Integration runtime boundary

The `aiconductor-runtime` integration prototype remains independent and is not
part of the normative AWHDL compiler pipeline. Milestone 1 does not compile
AWHDL into runtime configuration or execute an AWHDL design. Integration
belongs after a versioned IR is introduced.

## Deferred questions

- Keyword case sensitivity and Unicode identifier rules.
- Full expression grammar and overload resolution.
- Closing-name equality for entities and architectures.
- Multiple drivers and signal resolution.
- Generic maps, device port declarations, `if`, `case`, timers, budgets, and
  parallel blocks.
- Classification lattice, taint, declassification, and cloud-export policy.
