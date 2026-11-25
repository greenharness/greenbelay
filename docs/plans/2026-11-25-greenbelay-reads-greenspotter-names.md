---
status: done
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
- `GREENBELAY_ADR_LINT` set and non-empty: it is not read, and the full and
  the `--interdiff` brief each print one stderr notice (exit stays 0).

## Phases
- [x] Phase 1 — rename the spawn, the formats and the CI pin:
  `crates/greenbelay-process/src/review_brief.rs`,
  `crates/greenbelay-process/src/candidates_block.rs`,
  `crates/greenbelay-process/src/implementation_review.rs`, `src/main.rs`,
  `tests/review_brief.rs`, `tests/implementation_review.rs`,
  `docs/implementation-review.md`, `README.md`, `CHANGELOG.md`,
  `.github/workflows/ci.yaml`

## Verification
| Check | Input | Expected | Actual |
| --- | --- | --- | --- |
| Suite | `cargo test --workspace` on the edited tree, and the commit gate (clippy, tests) on both commits | pass | pass; `check_with_greenspotter` ran the real greenspotter 0.20.0 `ruling-record --check` on a record the new code wrote |
| New spawn on real input | `target/debug/greenbelay review-brief --plan <this plan>` on the committed tree, installed greenspotter 0.20.0 | a `<<<CANDIDATES sha=… n=…>>>` block | `n=3`, block present, exit 0 |
| Old variable | same, with `GREENBELAY_ADR_LINT=x`; and `--interdiff` | one stderr notice per brief, exit 0 | notice printed on the full brief by hand; both paths, and an empty value, by `the_retired_adr_lint_variable_gets_a_notice_in_the_full_and_the_delta_brief` |
| CI pin | `greenspotter sync --check` step | recorded after the run | see the PR checks |

## Decision log
- 2026-11-25: landed without a `## Review panel`: written after the edits as
  a programme ticket record; the implementation review is the check.

## Implementation review
- implementation-review: approve on the third pass; fixed the placeholder test name, the unticked phase, the missing real-input check, and the missing notice test.
- backend, lang:rust, unix: approve at delta 2; fixed the hook wording in the changelog, stale names, the notice on the `--interdiff` path and its text from `env_var!`.
- deliberate: the notice test's `full_brief_with` repeats the PATH setup that `interdiff_env` has (Rust, optional).
- deliberate: the CI-pin row reads "see the PR checks": CI cannot run before the push.
