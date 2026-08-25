# Getting started with AWHDL

[日本語（正本）](GETTING_STARTED_ja.md) | English reference translation

> The Japanese document is normative and takes precedence if the versions differ.

## Requirements

Milestone 1 requires only Rust and Cargo. It has no dependency on a particular
model, accelerator, editor, network service, filesystem layout, or API account.

From the `awhdl` directory:

```bash
cargo test --workspace
cargo run -p conductor-cli -- check examples/hello.awhdl
```

For machine-readable output:

```bash
cargo run -p conductor-cli -- check examples/hello.awhdl --output json
```

## First design

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

The entity declares the external interface. The architecture declares one
device and one process. A change to `task` activates the process, which invokes
the device and directs its result to the `result` port.

At Milestone 1, `aic check` verifies parsing, declaration structure, references,
and output targets. It does not invoke `llm` or any other device.

## Exit status

| Status | Meaning |
| --- | --- |
| 0 | The implemented checks passed. |
| 1 | The source file could not be read. |
| 2 | Parsing failed. |
| 3 | Structural checking failed. |

Passing Milestone 1 checks does not imply that the full v0.2 language or its
security policies have been validated. See
[Implementation status](IMPLEMENTATION_STATUS.md).
