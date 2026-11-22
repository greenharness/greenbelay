---
canonical: decisions:docs/plans/2026-11-21-approved-programmes-run-without-person.md
phases: [0]
status: done
---
Part of [Approved programmes run without the person](https://github.com/fredericrous/decisions/blob/main/docs/plans/2026-11-21-approved-programmes-run-without-person.md).

## Code review

- greenbelay: backend, lang:rust (unix, tui left out: no command-line output or flag changes)

## Verification

- Unit tests `implementation_review::` → the side transcript's review and profile review are read; another session's are not; the main transcript is read last; an unreadable main transcript is an error → passed. The first test would fail on the old code: its main transcript is empty.
- Hold cases → a later write wins; a side approve rewritten later still loses to a side rework; a same-file delta approve clears it; one agent's T1 rework, then T2 approve, appends nothing → passed.
- `an_agent_of_the_session_reviews_and_pushes_its_own_tree` through the real push hook, with `backend` declared → without the side file the push is not silent; with it, silent, `reviewed` journaled, pass filed → passed. The whole `tests/implementation_review.rs` suite and clippy `-D warnings` pass.
- Scratch run (not committed) of the new `reviews()` on the real 2026-11-15 session 32290a5c → the fork's side-transcript review is read (1 in that file; 41 reviews and 73 profile reviews for the session) → passed.
- A probe: a background `general-purpose` agent launches a nested agent, and its transcript lands in this session's `subagents/` → passed.

## Implementation review

- implementation-review: approve-with-changes over 4 rounds (the contract plans 2; each fix raised the next point). Fixed: same-line reason on a `let _`; a push-check-level test; side files in write order, a fail-closed hold, then narrowed to reworks another file approves (the full hold blocked cross-tree carries). 60k + 70k + 78k + 81k.
- backend: approve-with-changes, then approve on the hold. Deliberate: an unreadable side file is skipped without a note (no session or repo in the reader; filed for a note sink). 40k + 46k + 50k.
- lang:rust: approve-with-changes, then approve twice. Deliberate: the O(n²) key clones over a handful of reviews per file, and the fail-closed `matches!`. A dead agent's rework stays held until the tree changes: intended. 53k + 58k + 64k.
