# cvim boundary for AWHDL

[日本語（正本）](README_ja.md) | English reference translation

> The Japanese document is normative and takes precedence if the versions differ.

cvim is a thin Vim-oriented frontend. It may submit an `.awhdl` file to
`aic check`, display diagnostics, and later invoke compile/sim/run operations.

cvim must not parse AWHDL, make trusted scheduling decisions, grant project
permissions, bypass cloud-egress policy, or implement sandbox enforcement.
Those responsibilities belong to AI Conductor.

Milestone 1 usage:

```bash
aic check path/to/workflow.awhdl
aic check path/to/workflow.awhdl --output json
```
