# AWHDL security model

[日本語（正本）](SECURITY_MODEL_ja.md) | English reference translation

> The Japanese document is normative and takes precedence if the versions differ.

## Trust boundary

AWHDL treats models, agents, MCP servers, deterministic programs, cloud
services, and humans as devices. Devices do not form the trusted control plane.
Parsing, policy evaluation, scheduling, budget enforcement, sandboxing,
completion checks, and audit logging belong to AI Conductor.

## Intended invariants

1. Effective permission is the intersection of system, user, project,
   workflow, and device policy; a project cannot grant itself privileges.
2. Data classification is monotonic unless an explicitly trusted declassifier
   lowers it.
3. Cloud-bound calls pass through one egress authorization boundary.
4. Static acceptance never bypasses runtime egress inspection.
5. External and destructive actions require explicit policy authorization.
6. Autonomous loops are bounded independently of device output.
7. Restricted payloads are omitted from ordinary audit logs by default.

## Classification order

The full draft specification defines:

```text
public < internal < confidential < restricted < secret
```

The first security-checking milestone will initially implement:

```text
public < internal < restricted
```

Milestone 1 only records classification spelling in the AST. It does not
enforce these rules yet.

## Reporting security issues

Do not include secrets, private datasets, credentials, or restricted payloads
in a public report. Report the smallest reproducible source and describe the
policy decision that was expected and observed.
