---
status: active
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
- [ ] Phase 1 — make `ROW_IDS` a single `ROW_ID` and rewrite its doc comment,
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
| Unit | `cargo test -p greenbelay-guard greenspotter_written`: list whose only row is the old id | `Gate::Closed` | |
| E2E | `cargo test --test greenspotter_written`: fake amont lists the old id, edit `src/a.rs` | stdout and stderr empty, exit 0, no greenspotter run | |
| Suite | `cargo test --workspace` | every `test result:` line reads `0 failed` | |
| Census | `greengate list --json` on each fleet repository's `origin/main`, before release | none lists `pre-commit-adr-lint-check` | |
| Gone | grep the old id in `crates tests src docs/rules.md` | matches only the two closed-gate tests' fixtures | |

<!-- panel: repos=greenbelay adds= reviewers=backend,language,tui,unix body-sha=91dc07f18b35 -->
