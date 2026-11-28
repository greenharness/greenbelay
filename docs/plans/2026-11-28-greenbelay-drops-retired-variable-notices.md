---
status: active
branch: chore/t28-rename-identifiers
repos: [greenbelay]
adrs: [ADR-0072, ADR-0073, ADR-0012]
plan-id: p-01M8FW72DA5C3SY245Y9NEJYHF
---
# greenbelay drops the retired variable notices

## Review panel
👉 **Decide:** none — approve if dropping a notice for variables nothing sets, with tests that set them and see nothing, is the whole slice.
📍 greenbelay · 2 phases · 1 release · next: Phase 1. Panel: backend, lang:rust, tui, unix (opus), codex.
**Changed by review:** marker-file fakes prove nothing is spawned; set-vs-unset runs compared byte for byte; Phases 1–2 merged.
**Verdicts:** 4 approve-with-changes, then 3 approve and backend approve-with-changes; 3 carried; words 472 → 596 (×1.26, fixes to unverifiable checks); remove 0, merge 1.
📄 Full reviews: [2026-11-28-greenbelay-drops-retired-variable-notices.reviews.md](2026-11-28-greenbelay-drops-retired-variable-notices.reviews.md)

## Picture

```
 review-brief env            6.18.2                        6.18.3
 ----------------            ------                        ------
 GREENBELAY_AVAL=x    --->   not read; stderr notice  --->  not read; silent
 GREENBELAY_ADR_LINT=x --->  not read; stderr notice  --->  not read; silent
 GREENBELAY_GREENTOPO  --->  read                     --->  read (unchanged)
```

Nothing sets the old variables, so the notice that named them goes; the
tests that set them and see nothing stay.

## Goal
decisions topo-spotter-rename t28, greenbelay slice. `review-brief` stops
naming `GREENBELAY_AVAL` and `GREENBELAY_ADR_LINT`: the notice that one of
them is still set goes, because nothing sets them any more. The variables
stay unread, and the tests that prove it stay as tests forbidding the old
names. Released as greenbelay 6.18.3 and installed on this Mac.

## Non-goals
- `greenspotter_written.rs`'s old row-id refusal test and the World env that
  sets `GREENBELAY_ADR_LINT` in every hook run: they forbid the old name
  (t21's allowlist).
- Recorded fixtures (`tests/fixtures/block/*.diff`, the guard corpus) and
  `tests/block.rs`'s fixture names: history.
- CHANGELOG, `docs/plans`, `docs/programmes`.

## Code review
- greenbelay: backend, lang:rust, unix (tui left out: it only removes one stderr line and adds no output)

## Behaviour
- Evidence that nothing sets them (2026-11-28): `env` and `launchctl getenv`
  print neither; the chezmoi source (`git grep` at HEAD) has no hit; every
  checkout under `~/Developer/Perso` holds them only in plans and
  programme tickets.
- `greenbelay review-brief` (full and `--interdiff`) with
  `GREENBELAY_AVAL=/nonexistent/aval` or `GREENBELAY_ADR_LINT=x` set prints
  nothing about them on stderr, exits as it does with them unset, and spawns
  neither value.
- With both unset, output is byte-for-byte what 6.18.2 prints.

## Phases
- [ ] Phase 1 — in one commit, remove `retired_program_notice` and
  `retired_notices` and their two call sites (full brief, delta brief), and
  flip the two notice tests into
  `the_retired_adr_lint_variable_is_not_read_or_named` and
  `the_retired_aval_variable_is_not_spawned_or_named`. Each sets the
  variable to a scratch executable that writes a marker file, in the full
  and the `--interdiff` brief, and asserts: the marker is absent, stderr
  contains neither `GREENBELAY_AVAL` nor `GREENBELAY_ADR_LINT`, and stdout,
  stderr and exit code equal the same run with the variable unset. The old
  test's empty-value branch goes (nothing tells empty from unset any more):
  `crates/greenbelay-process/src/review_brief.rs`, `tests/review_brief.rs`
- [ ] Phase 2 — release 6.18.3 through the `tag-release` skill, install it,
  print `greenbelay --version`: `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`

## Decision log
- 2026-11-28 — flip the tests rather than delete them, as t22 did for the old
  row id, because a test that sets the old variable and sees nothing is the
  proof the Destination asks for ("no live program reads `aval`").
- 2026-11-28 — patch release: no flag, key or exit code changes; a stderr
  notice for a variable nothing sets is dropped.

## Verification
- Phase 1: `cargo test --test review_brief -- --exact
  the_retired_adr_lint_variable_is_not_read_or_named
  the_retired_aval_variable_is_not_spawned_or_named` → 2 passed; each fails
  when `notices.extend(retired_program_notice())` is put back at the full
  and then the delta call site → <result>
- Phase 1: piloted `greenbelay review-brief -C <scratch repo> --base HEAD~1`
  with `GREENBELAY_AVAL=/nonexistent/aval GREENBELAY_ADR_LINT=x`, full and
  `--interdiff` → exit 0, stderr names neither variable, stdout `diff`s
  empty against the same run unset → <result>
- Phase 1: with both unset, `cmp` stdout and stderr of 6.18.2 and the branch
  build on the same scratch repo, full and `--interdiff` → identical →
  <result>
- Phase 2: `greenbelay --version` → `greenbelay 6.18.3` → <result>
- Census, on `HEAD` before the push and on `origin/main` after the merge,
  outside CHANGELOG and `docs/`: `git grep -nIiE
  '(^|[^[:alpha:]])aval|adr-lint|adr_lint'` plus case-sensitive
  `git grep -nE '[a-z]Aval'` → only `tests/greenspotter_written.rs`,
  `crates/greenbelay-guard/src/rules/greenspotter_written.rs`'s row-id
  test, the two flipped tests in `tests/review_brief.rs`, `tests/block.rs`
  fixture names, `tests/fixtures/block/*.diff` and the guard corpus →
  <result>

## Outcome
<!-- panel: repos=greenbelay reviewers=backend,lang:rust,tui,unix,codex body-sha=c6f652667256 -->
