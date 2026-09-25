# AWHDL language specification

[日本語（正本）](LANGUAGE_SPEC_ja.md) | English reference translation

> **Source of truth.** Normative: [LANGUAGE_SPEC](LANGUAGE_SPEC.md),
> [RUNTIME_SPEC](RUNTIME_SPEC.md), [SECURITY_SPEC](SECURITY_SPEC.md),
> [IR_SPEC](IR_SPEC.md) and [PROFILE_v0.1](PROFILE_v0.1.md). Non-normative:
> [IMPLEMENTATION_GUIDE](IMPLEMENTATION_GUIDE.md), examples, tutorials,
> migration notes and status reports. On conflict, PROFILE_v0.1 wins, then the
> normative specs. Only PROFILE_v0.1 defines the v0.1 implementation scope.
> This English document is a reference translation; the Japanese document is
> normative and takes precedence if the versions differ.

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
device name : declassifier generic (from => <class>, to => <class>,
    [filter => <deterministic device>, filter_method => <method>,] [approval => human]);
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
    signal <= declassify <expr> using <declassifier>;
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
`device.done`). Keywords are reserved and need word boundaries, except that
`timeout` may be written as the member in the sensitivity name `device.timeout`.

A sensitivity name must be an event the runtime raises: `signal` /
`signal.changed`, `timer`, `barrier` / `barrier.ready`, or `device.done` /
`device.failed` / `device.timeout`. Any other name (a bare `device`,
`device.completed`, `signal.field`, `timer.ready`, ...) would never wake the
process and is `E206`.

Static semantics (checker diagnostics): `E2xx` structure (unknown or duplicate
names, writes to `in` ports, invalid budget keys or non-positive times, barrier
members that are not signals, two parallel branches writing one signal);
`E3xx` information flow (explicit flows `E301`-`E303`, implicit flows
`E304`-`E306`; see SECURITY_SPEC); `E4xx` constructs that the language
defines but PROFILE_v0.1 excludes (`confidential`, `secret`, `sandbox`,
`private_cloud`, `external`) or that are unknown.

### Declassification (implemented, 2026-09-25)

Only a `declassify` statement using a `declassifier` device may lower a class.
A declassifier declares the range it may release (`from` to `to`) and how a
release is authorized. Choose at least one means; when both are given, both are
required:

- `filter => <device>`, `filter_method => <method>`: a deterministic check by a
  `deterministic` device located `local`.
- `approval => human`: human approval by the runtime's configured approver
  (HumanPort in approver mode), who sees the content to be released.

```vhdl
device pii_check : deterministic generic (location => local, route => "pii_filter");
device release : declassifier generic (from => restricted, to => internal,
    filter => pii_check, filter_method => scan, approval => human);
...
summary <= declassify draft using release;
```

Static rules (checker):

- `E219`: an invalid declassifier declaration: `from` or `to` missing, `to` not
  weaker than `from`, no means, a `filter` that is not a `local` `deterministic`
  device, a `filter` without `filter_method`, an `approval` other than `human`,
  or a declassifier whose `location` is not `local`.
- `E220`: the device after `using` is not a declassifier, or a declassifier is
  called like an ordinary device.
- `E307`: the released expression's class is stronger than the declassifier's `from`.
- A release has class `to`. Storing it in a signal weaker than `to` is `E303`;
  a context stronger than the target is `E304`.
- Whether a release succeeds depends on its content, so the declassifier's
  `done` and `failed` events have the strongest of the released expression's
  class and the context class (SECURITY_SPEC, implicit flows).

`case`, `await`, `type` / FSM declarations, `retry`, `approve`, `policy`,
`export`, `configuration` and temporal assertions are not in v0.1 and do not parse.
