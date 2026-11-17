---
canonical: greenharness/greengate docs/plans/2026-11-16-detached-land.md
plan-id: p-01M7JDWHTXSAA7MEMGETF65JSW
phases: [2]
status: active
---
# B2: a detached land with a receipt

Pointer to the canonical plan in greengate. This repository carries Phase 2:
the land guard reads `--detach` and `--wait`, the landing assertion defers a
`running` receipt, the version probe is shared, and the `land-detached` rule.
The canonical plan's 2026-11-17 delta (body e5bea13218f7) moved Phase 2 to
greenbelay after the rename.

## Code review
- amont-agent: backend, lang:rust (unix, tui left out: a hook rule and an assertion, no command-line flag or output)
