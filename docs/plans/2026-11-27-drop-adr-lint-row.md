---
status: done
branch: chore/t22-drop-adr-lint-row
repos: [greenbelay]
adrs: []
plan-id: p-01M8D1NT9DXVX5TM38QNEQBQ11
---
# Drop the old adr-lint row

## Review panel
👉 **Decide:** none — approve if the census (no repository lists the old row) holds.
📍 greenbelay · 1 phase · 1 release · next: Phase 1. Panel: backend, lang:rust, tui, unix (opus).
**Changed by review:** tests flipped to assert a closed gate; one phase; census and stderr checks added.
**Verdicts:** 4 approve after round 2; 0 carried; words 187 → 290; remove 0, merge 1.

## Goal
The write-time hook's gate (`greenspotter-written`) stops accepting the old row
id `pre-commit-adr-lint-check` (programme topo-spotter-rename t22). A census on
2026-11-27 found no repository whose `greengate.conf` still lists it, vendored packs included (`greengate list --json`).

## Non-goals
- The retired `GREENBELAY_AVAL` / `GREENBELAY_ADR_LINT` notices, recorded
  fixtures and the changelog history stay (t21 allowlist).
- The `greengate-list-v1` / `amont-list-v1` envelope names stay.

## Code review
- greenbelay: backend, lang:rust, unix (tui left out: no output change)

## Behaviour
- The gate opens only on `pre-commit-greenspotter-check`; a repository listing
  only the old id leaves the gate closed, as any repository without the row
  (nothing journalled, `src/hook.rs` `Gate::Closed => false`).

## Phases
- [x] Phase 1 — make `ROW_IDS` a single `ROW_ID` and rewrite its doc comment,
  flip its unit assertion to `Gate::Closed` for the old id, turn the e2e test
  `a_repository_vendoring_the_old_adr_lint_row_still_opens_the_gate` into
  `a_repository_vendoring_only_the_old_adr_lint_row_is_closed` (greenspotter not
  run), drop the old id from the rule's line in `docs/rules.md`, and add the
  changelog entry for the patch release:
  `crates/greenbelay-guard/src/rules/greenspotter_written.rs`,
  `tests/greenspotter_written.rs`, `docs/rules.md`, `CHANGELOG.md`

### 🧑 Your steps
- none

## Verification
| Check | Input | Expected | Actual |
| --- | --- | --- | --- |
| Unit | `cargo test -p greenbelay-guard greenspotter_written`: list whose only row is the old id | `Gate::Closed` | pass: `cargo test -p greenbelay-guard greenspotter_written` ok |
| E2E | `cargo test --test greenspotter_written`: fake amont lists the old id, edit `src/a.rs` | stdout empty, exit 0, no greenspotter run, nothing journalled | pass: `cargo test --test greenspotter_written` ok |
| Suite | `cargo test --workspace` | every `test result:` line reads `0 failed` | pass: `cargo test --workspace` exit 0, 1256 s; pre-commit gate suite ok (169 passed, 0 failed) |
| Census | `greengate list --json` on each fleet repository's `origin/HEAD` (the default branch), before release | none lists `pre-commit-adr-lint-check` | 0 hits in `greengate.conf`, `amont.conf` and `*.pack` at `origin/HEAD` of 74 fleet repositories, 2026-11-27 |
| Gone | grep the old id in `crates tests src docs/rules.md` | matches only the two closed-gate tests' fixtures | two lines: the unit fixture and the e2e fixture; none in `src` or `docs/rules.md` |

## Decision log
- 2026-11-27: the e2e row drops "stderr": the hook writes nothing to stderr and the helper does not capture it; a journal-empty assertion was added instead.

## Implementation review
- implementation-review: approve-with-changes at delta 1; fixed the journal assertion, dropped the unprovable stderr claim, filled Actuals and ticks; deliberate: the journal assertion cannot tell gate-open (no open note exists), `runs()` carries that. About 110k tokens.
- backend: approve; no findings; carried one hand-check, the census row names the default branch only (worktrees' own branches were not scanned).
- lang:rust: approve; one low nit (`id == ROW_ID` closure vs `== Some(ROW_ID)`), deliberate: kept as is.
- unix: approve; no findings.

<!-- panel: repos=greenbelay adds= reviewers=backend,language,tui,unix body-sha=91dc07f18b35 -->
