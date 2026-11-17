---
status: active
plan-id: p-01M7M5THKW6CY49NCHMFMHC1JJ
canonical: relais:docs/plans/2026-11-17-sessionstart-on-clear-shows-resume.md
phases: 2, 3 (greenbelay's release)
branch: feat/resume-on-clear
repos: [relais, amont-agent, decisions]
adrs: [ADR-0062, ADR-0075]
---
# SessionStart on clear shows how to resume (pointer)

The canonical plan is in relais. This repository carries Phase 2: the
greenbelay inbox notice at a `startup` or `clear` session start. It also
carries greenbelay's share of Phase 3, its release.

## Code review

- amont-agent: backend, lang:rust, unix, tui
