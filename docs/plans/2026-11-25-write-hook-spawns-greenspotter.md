---
status: done
plan-id: p-t15-greenspotter-hook
branch: feat/greenspotter-hook
repos: [greenbelay]
adrs: []
---
# Write hook spawns greenspotter

## Goal
The write-time hook runs `greenspotter check --json -- <path>` in place of
`adr-lint check`, under the rule id `greenspotter-written`, so the machine
switch (topo-spotter-rename t09) can remove `adr-lint` from PATH without the
hook going quiet with `adr-lint-unavailable`.

## Non-goals
- No alias for `GREENBELAY_ADR_LINT` in the hook (the hook prints nothing to
  stderr; review-brief already prints the retired-variable notice).
- No migration of old `adr-lint-written` journal rows: they stay as history.
- No change to `greengate.conf`'s vendored adr-lint pack in this repository
  (the fleet sweep, t10).

## Code review
- greenbelay: backend, lang:rust, unix

## Behaviour
- `GREENBELAY_GREENSPOTTER` names the program, else `greenspotter` from PATH.
- The gate opens on `pre-commit-greenspotter-check` (greenspotter 0.20.0's
  `greengate.pack` row `greenspotter-check`), and still on
  `pre-commit-adr-lint-check` until the fleet sweep renames vendored packs.
- Journal keys: `greenspotter-written`, `greenspotter-slow`,
  `greenspotter-unavailable`, `greenspotter-exit-{2,3,other}`,
  `greenspotter-unparsed`.
- The advice reads `greenspotter found …` and the waiver hint
  `greenspotter: allow <rule-id> -- <reason>` (greenspotter 0.20.0's marker).

## Phases
- [x] Phase 1 — rename the rule, its spawn, journal keys, corpus and tests:
  `crates/greenbelay-guard/src/rules/greenspotter_written.rs`,
  `crates/greenbelay-guard/tests/corpus/greenspotter-written.cases`,
  `tests/greenspotter_written.rs`, `src/hook.rs`, `src/rule_table.rs`, the
  guard's `rollup.rs`, `backtest.rs`, `corpus.rs`, `doctor.rs`,
  `settings.rs`, `payload.rs`, `rules/mod.rs`, `tests/doctor.rs`,
  `docs/rules.md`, `CHANGELOG.md`

## Verification
| Check | Input | Expected | Actual |
| --- | --- | --- | --- |
| Suite | `cargo test --workspace` on the final code tree (8d35ee12) | pass | pass, exit 0, 1152 s; a mutant hook falling back to `GREENBELAY_ADR_LINT` failed `the_retired_adr_lint_variable_is_not_read` |
| Real spawn | the built hook on a PostToolUse Write payload of a `.rs` file in a scratch repo whose `greengate list --json` lists `pre-commit-greenspotter-check`, with installed greenspotter 0.20.0 and no `adr-lint` on PATH | greenspotter runs; no `greenspotter-unavailable` row | journal `greenspotter-written gate-open`; greenspotter exit 0 (`[]`); no unavailable row; adr-lint absent from PATH |
| Real spawn, old row | same, in a scratch repo whose `greengate list --json` lists only `pre-commit-adr-lint-check` | the gate opens; greenspotter runs | journal `gate-open`; greenspotter exit 0; no unavailable row |

## Decision log
- 2026-11-25: landed without a `## Review panel`: written as a programme
  ticket record (t15), like t13; the implementation review is the check.
- 2026-11-25: the old row id still opens the gate, so repositories the sweep
  has not reached keep their write-time check after the release.

## Implementation review
- implementation-review: approve-with-changes at delta 2 (plan record only, done here); fixed the missing old-row fallback, `w` names, pending rows; 146k tokens, 104 s.
- backend: approve at delta 2; fixed the fallback, its unit and e2e tests, and a retired-variable test that now unsets the override (mutant-checked); 136k tokens, 75 s.
- lang:rust, unix: approve at delta 1; fixed the fallback, the `al` alias, leftover wording and test ids.
- deliberate: `Verdict::Failed(&'static str)` stays stringly typed (predates the rename; outside the ticket).
- deliberate: `ROW_IDS` keeps `pre-commit-adr-lint-check` until the fleet sweep (t10) removes it.
