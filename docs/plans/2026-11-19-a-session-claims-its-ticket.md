---
canonical: greenharness/relais docs/plans/2026-11-19-a-session-claims-its-ticket.md
plan-id: p-01M7SH79WA5FGSCQK2F7K609NZ
phases: [3]
status: active
---
# A session claims its ticket, and a handoff names only an unheld one

Pointer to the canonical plan in relais (session-boundaries t26). This
repository carries Phase 3: the clear-relay plugin records, on each `submit`
line, the ticket a `resume <repo> <programme> <ticket>:` prompt names, or
`null`. It refuses nothing; the week's count reads the field.

## Code review
- amont-agent: backend, lang:typescript (unix and tui left out: no CLI output)
