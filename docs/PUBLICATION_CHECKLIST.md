# Publication checklist

[日本語（正本）](PUBLICATION_CHECKLIST_ja.md) | English reference translation

> The Japanese document is normative and takes precedence if the versions differ.

Before publishing an AWHDL revision:

1. Confirm the README identifies the specification as a draft where
   applicable.
2. Keep provider and product names non-normative or use reserved example
   placeholders.
3. Do not publish credentials, private endpoints, personal usernames, absolute
   home paths, private project paths, generated runtime state, or build output.
4. Ensure examples run without proprietary software or cloud credentials.
5. Keep `IMPLEMENTATION_STATUS.md` aligned with tested behavior.
6. Mark Japanese documents as normative and English documents as reference
   translations; do not let an English change precede its Japanese counterpart.
7. Verify that specification copies in the root and `docs/` have equivalent
   content and valid location-relative links.
8. Run formatting, tests, and Clippy for the complete workspace.

Suggested environment-specific content scan:

```bash
rg -n '/home/[^/ ]+|/Users/[^/ ]+|192\.168\.|10\.[0-9]+\.|api[_-]?key|private[_-]?key' \
  --glob '!target/**'
```

Matches in generic examples must be reviewed manually; a match is not by
itself proof of a disclosure.
