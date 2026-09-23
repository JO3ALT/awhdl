# AWHDL language specification

> **Source of truth.** Normative: [LANGUAGE_SPEC](LANGUAGE_SPEC.md),
> [RUNTIME_SPEC](RUNTIME_SPEC.md), [SECURITY_SPEC](SECURITY_SPEC.md),
> [IR_SPEC](IR_SPEC.md) and [PROFILE_v0.1](PROFILE_v0.1.md). Non-normative:
> [IMPLEMENTATION_GUIDE](IMPLEMENTATION_GUIDE.md), examples, tutorials,
> migration notes and status reports. On conflict, PROFILE_v0.1 wins, then the
> normative specs. Only PROFILE_v0.1 defines the v0.1 implementation scope.

Scope: syntax, type system, static semantics and language constructs of AWHDL.
This document does not define the implementation scope; a construct described
here but absent from, or marked `v0.2` in, PROFILE_v0.1 is unsupported in v0.1.

## Normative text

The language is specified by the AWHDL Language Specification v0.2:
[`AWHDL_LANGUAGE_SPEC_v0.2_ja.md`](AWHDL_LANGUAGE_SPEC_v0.2_ja.md)
(Japanese, normative) with the English reference translation
[`AWHDL_LANGUAGE_SPEC_v0.2.md`](AWHDL_LANGUAGE_SPEC_v0.2.md).
The root `AWHDL_LANGUAGE_SPEC_v0.1.md` is
historical design input and is superseded by v0.2.

## Precedence over the language text

- Execution semantics (identity, generation, Value / Event / Invocation,
  delivery, retry, Effects, checkpoints, completion evaluation) are defined by
  [RUNTIME_SPEC](RUNTIME_SPEC.md). Where the language text describes runtime
  behavior differently, RUNTIME_SPEC wins.
- Classification, taint, capability, approval and audit are defined by
  [SECURITY_SPEC](SECURITY_SPEC.md).
- Serialized forms are defined by [IR_SPEC](IR_SPEC.md).

## Constructs whose semantics are fixed but syntax is open

The refinement procedure fixes semantics before syntax for these constructs.
Their source syntax is not yet decided, and no parser may accept a provisional
spelling until this section names it:

- Event declarations and `.changed` / `.completed` sensitivity
  (semantics: RUNTIME_SPEC, Value, Event and Invocation).
- `completion when hard { ... } soft { ... };` or `require hard` / `prefer`
  (semantics: RUNTIME_SPEC, Completion). Conditions are currently configuration.
- Effect class, approval and capability annotations on devices (semantics:
  RUNTIME_SPEC Effects and SECURITY_SPEC). These are currently configuration.

## v0.1 syntax (implemented)

These spellings from the v0.2 language text are fixed for v0.1 and accepted by
the parser (`awhdl-parser/src/awhdl.pest`). Anything else is a syntax error.

```vhdl
device name : agent | mcp | deterministic generic (key => value, ...);
    -- keys used by v0.1: route => "<configured action>", location => local | cloud,
    -- clearance => public | internal | restricted
signal a, b : type<class> := <constant>;
timer name : period <n> ms|sec|min|hour|day;
budget name is iterations <= n; wall_time <= <time>; tool_calls <= n; model_calls <= n; end budget;
barrier name (signal, ...);

assert always (<expr>);
assert never (<expr>);
assert never (<class> -> <location>);        -- static flow rule

process(name, name.changed, device.done, device.failed, device.timeout, timer, barrier.ready)
    timeout <time>;                          -- optional
begin
    device.method(<expr>, ...) [timeout <time>] -> signal;
    signal <= <expr>;
    if <expr> then ... elsif <expr> then ... else ... end if;
    parallel device.method(...) -> signal; ... end parallel;
    assert <expr>;
    null;
on timeout                                   -- optional
    ...
end process;
```

Expressions, loosest to tightest: `or`; `and`; `not`; one comparison
(`=`, `/=`, `<`, `<=`, `>`, `>=`); `+` / `-` on integers; literals (string,
integer, `true` / `false`, time) and names (`signal`, `signal.field`,
`device.done`). Keywords are reserved and need word boundaries.

Static semantics (checker diagnostics): `E2xx` structure (unknown or duplicate
names, writes to `in` ports, invalid budget keys or non-positive times, barrier
members that are not signals, two parallel branches writing one signal);
`E3xx` information flow (see SECURITY_SPEC); `E4xx` constructs that the language
defines but PROFILE_v0.1 excludes (`confidential`, `secret`, `sandbox`,
`private_cloud`, `external`, `declassifier`) or that are unknown. `case`,
`await`, `type` / FSM declarations, `retry`, `approve`, `policy`, `export`,
`configuration` and temporal assertions are not in v0.1 and do not parse.
