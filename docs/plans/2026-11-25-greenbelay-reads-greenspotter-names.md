---
status: active
plan-id: p-t13-greenspotter
branch: feat/greenspotter-names
repos: [greenbelay]
adrs: []
---
# Greenbelay reads greenspotter names

## Goal
`review-brief` asks `greenspotter` (not `adr-lint`) for the candidates of a
branch, reads `greenspotter.candidates/1`, and writes `greenspotter.ruling/1`
records, so the machine switch (topo-spotter-rename t09) can remove `adr-lint`
from PATH. CI pins greenspotter 0.20.0.

## Non-goals
- The write-time `adr-lint-written` hook (`src/hook.rs`) still runs
  `adr-lint check`; it is a later ticket.
- No alias for `GREENBELAY_ADR_LINT`, no reading of `adr-lint.candidates/1`.
- No release in this change.

## Code review
- greenbelay: backend, lang:rust, unix

## Behaviour
- `GREENBELAY_GREENSPOTTER` names the program, else `greenspotter` from PATH.
- A document with another format token is refused as before.
- Old `adr-lint.ruling/1` rows stay as history.

## Phases
- [ ] Phase 1 — rename the spawn, the formats and the CI pin:
  `crates/greenbelay-process/src/review_brief.rs`,
  `crates/greenbelay-process/src/candidates_block.rs`,
  `crates/greenbelay-process/src/implementation_review.rs`, `src/main.rs`,
  `tests/review_brief.rs`, `tests/implementation_review.rs`,
  `docs/implementation-review.md`, `README.md`, `CHANGELOG.md`,
  `.github/workflows/ci.yaml`

## Verification
`cargo test --workspace` passes (it did on the edited tree, 2026-11-25); CI's
`greenspotter sync --check` step concludes `success`.

## Decision log
- 2026-11-25: landed without a `## Review panel`: written after the edits as
  a programme ticket record; the implementation review is the check.
