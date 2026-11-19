---
canonical: greenharness/relais docs/plans/2026-11-19-a-session-claims-its-ticket.md
plan-id: p-01M7SH79WA5FGSCQK2F7K609NZ
phases: [3]
status: done
---
# A session claims its ticket, and a handoff names only an unheld one

Pointer to the canonical plan in relais (session-boundaries t26). This
repository carries Phase 3: the clear-relay plugin records, on each `submit`
line, the ticket a `resume <repo> <programme> <ticket>:` prompt names, or
`null`. It refuses nothing; the week's count reads the field.

## Code review
- amont-agent: backend, lang:typescript (unix and tui left out: no CLI output)

## Verification (Phase 3, 2026-11-19)

`claude plugin validate` passes; `claude plugin test` runs 30, 30 pass. The
checks, each driven through the plugin's own engine (answer → 10 s → submit):

| Input (the handoff prompt) | Expected `named` | Actual |
|---|---|---|
| `continue with Phase 2 of docs/plans/x.md` | `null` | `null` |
| `resume sre-agent cve-lane t20: bump the shared pin` | sre-agent / cve-lane / t20 | as expected |
| `resume decisions session-boundaries t26-sessions-claim-their-work: …` | ticket `t26-sessions-claim-their-work` | as expected |
| `resume decisions agents-many-nodes an-t02-primary-node: …` | ticket `an-t02-primary-node` | as expected |
| `resume homelab fix bug: the flapping link` | `null` | `null` |
| `safe to /clear; parked: next due 2026-11-24` | no arm, no submit | none |

All 321 ticket file stems under `~/Developer/Perso/*/docs/programmes` match
the ticket shape (`grep -Evc` gave 0).

## Implementation review

- implementation-review: approve-with-changes ×5; fixed: ticket ids as file stems (blocking), prefixed ids, the `holds-until:` marker, the Verification entry, status. 79k–99k, 29–92 s.
- backend: approve after the delta (ticket-shaped `named`, test naming, rules.md). 54k + 57k.
- lang:typescript: approve after the delta (ticket group, named groups, park test name). 44k + 48k.

## Decision log

- 2026-11-19: `named` matches a ticket id, not any third word as the
  canonical plan's `(\S+)` would: an ordinary `resume homelab fix bug:` prompt
  would otherwise count as a named ticket and skew the week's count. The id is
  the file stem relais prints, with an optional lowercase prefix.
