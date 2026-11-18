---
status: active
plan-id: p-01M7QPAQ11ZKVEGSFJAXP1Q7RE
canonical: relais:docs/plans/2026-11-18-resume-outside-a-repository.md
phases: 2 (greenbelay's inbox at a non-repository cwd, and its release)
branch: feat/resume-outside-a-repository
repos: [relais, amont-agent, decisions]
adrs: [ADR-0075, ADR-0077]
---
# A cleared session outside a repository still sees how to resume (pointer)

The canonical plan is in relais. This repository carries Phase 2: at a
`startup` or `clear` whose cwd is not inside a git repository, greenbelay
prints the inbox notice of each direct child main checkout that has
unfiled lines, each after an `in <name>: cd <path>` line. It also carries
greenbelay's release.

## Code review

- amont-agent: backend, lang:rust, unix, tui, architect
