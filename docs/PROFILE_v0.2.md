# Profile v0.2 (planning)

[日本語（正本）](PROFILE_v0.2_ja.md) | English reference translation

> **Planning document, not normative for v0.1.** The single status record for
> every feature, including those planned for v0.2, is the matrix in
> [PROFILE_v0.1](PROFILE_v0.1.md). This page only groups the rows whose
> profile is `v0.2` and states the entry conditions for moving them.

## Candidate features

Rows marked `v0.2` in the PROFILE_v0.1 matrix: declassifier, HTTP MCP transport,
per-host network policy enforcement, cloud DLP / egress inspection, taint /
information-flow check, advanced cancellation, CLI resume and run lock,
transaction / time-limited approval scopes, and automatic provider
reconciliation lookup.

## Entry conditions

- v0.1 is complete: every v0.1 row is conformant.
- Each feature moved into scope gets a DESIGN_DECISIONS entry, normative text in
  the owning spec, and a matrix row update in the same change.
- Features touching external writes keep the Effect, idempotency, approval and
  capability rules; none may weaken them.
