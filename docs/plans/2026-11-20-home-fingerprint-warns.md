---
status: active
plan-id: p-01M7W0WBH0ABNE7ZWF4JH0S6RN
canonical: relais:docs/plans/2026-11-20-home-fingerprint-warns.md
phases: 2 (greenbelay assertion home-fingerprint-drift)
branch: feat/home-fingerprint
repos: [relais, greenbelay]
adrs: []
---
# Home fingerprint: warn when a test command changes sensitive files (pointer)

The canonical plan is in relais; Phase 1 (relais) merged as relais#512. This
repository carries Phase 2: the advise-only assertion `home-fingerprint-drift`,
which snapshots the watched set before a test/build command and warns after it
when a file changed. It never refuses.

## Code review

- greenbelay: backend, lang:rust, unix, tui
