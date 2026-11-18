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

## Verification

- Built-binary probe at `/Users/fredericrous/Developer/Perso` (SessionStart `clear`) → expected: an amont notice under `in amont: cd …` → actual: `in amont: cd '/Users/fredericrous/Developer/Perso/amont'` + its 1-line notice, 1.06 s wall.
- `cargo test --workspace` + clippy at each commit's gate → passed (28 checks). The integration tests cover two child inboxes, a `.git` in a parent dir (no scan), a newline name (skipped), the overflow, and a filing-only child.

## Implementation review

- implementation-review: approve-with-changes → approve on the Delta. Fixed: control-char names skipped, scan only when no ancestor `.git`. Pack refresh is its own commit. 62k+36k, 63+104 s.
- backend: approve-with-changes on both rounds. Fixed: ancestor check, pure `outside_notice` with a 7-child test, singular. Deliberate: a `date` call before the child check (children exist at the root). 67k+31k.
- lang:rust: approve-with-changes on both rounds. Fixed: `cfg(unix)` gates, `has_unfiled`, one stance/day. Deliberate: tuple → struct and the redundant text guard, left for a tidy. 56k+30k.
- unix, tui, architect: unix and tui approve-with-changes on both rounds; architect a-w-c → approve. Deliberate: a non-UTF-8 cwd, an 80-column overflow, a silent skip of unprintable names (documented), migrate_plugin's quoter left. 54k+31k, 51k+31k, 56k+31k.

Phase 2's code is done; next: greenbelay's release after the merge.
