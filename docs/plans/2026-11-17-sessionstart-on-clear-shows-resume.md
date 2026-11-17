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

## Implementation review

- implementation-review: approve-with-changes, then approve on the delta. Blocking test-rule hits fixed (checked removals, `reply`, no banner); the fresh-line wording follows carried item 3, deliberate. 74k, 92 s + 17 s.
- backend: approve-with-changes, then approve. The take keeps its mtime age, and `inbox-read/1` says the filer touches it after the move (ctime cannot be set by a test). A "0 lines unfiled" header for a young take alone is filed as an inbox follow-up. 52k, 41 s + 22 s.
- lang:rust: approve-with-changes, then approve. `notice(inbox, stance, today, now)` reads only the files it is given. `push_str(&format!)` is kept over `let _ = write!`, deliberate. 48k, 41 s + 22 s.
- unix: approve-with-changes on both rounds. `date` now has a 500 ms deadline. Deliberate: `git rev-parse` uses the same unbounded helper as the other session notices. 65k, 47 s + 19 s.
- tui: approve-with-changes, then approve. The text says where to file an orphan, counts only unfiled lines, and says "session id". 44k, 22 s + 14 s.

Phase 2 is done. Phase 3, greenbelay's release, is next.
