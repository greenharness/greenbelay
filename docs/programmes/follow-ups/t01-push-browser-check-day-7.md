+++
type = "task"
mode = "afk"
status = "closed"
blocked_by = []
repos = ["greenbelay"]
due = "2026-11-28"
+++
# Does push-browser-check graduate to deny, demote, or stay at advise

## Question

The day-7 check of `docs/plans/2026-11-20-push-browser-check.md`: seven days
after skills 0.19.0 was installed on this machine (2026-11-21, grh 0.14.2),
what did the rule say on UI pushes, and does that meet the plan's bars?

## Look at

- `~/.claude/greenbelay/journal.log`, the `push-browser-check` lines dated
  on or after 2026-11-21, matched on the outcome field exactly:
  - passes: `unconfirmed bound` (a check in this session), `unconfirmed
    remembered` (a tree a pass file already covers: a re-push after a
    push-preview hold, a failed publish or in a later session) and
    `unconfirmed declared` (the plan's `## Browser check` escape; the
    separate `- declared` line the rule also writes is the same push, not a
    second pass);
  - advices: `advise advised … check=missing|stale|unreachable|unknown`;
    `check=gap` is a machine without the agent, reported beside the ratio
    and left out of it;
  - neither: `unconfirmed delete-only|dry-run|no-ui-repository|no-interface-change|…`
    and the `deny denied` lines of a refused store write.
  The ratio is passes over passes plus advices.
- Among the advices, how many carry `check=unreachable`, and how many pushes
  were advised twice on the same tree (an advise the agent could not
  satisfy). Beside them, the number of `remember-failed` lines: each is a
  tree a later session had to check again because the store could not be
  written.
- `greenbelay corpus check` (read-only: it prints every rule's row, where
  `push-browser-check` should read 17 reviewed, 9 negative, above the 12/4
  floor, and replays every reviewed judgement). `graduate` itself has no dry
  mode: when the corpus passes it moves the rule at once, so it is run only
  in the answer below, after the bars are met.

## Answer looks like

- ≥70% of UI pushes `bound`, `remembered` or `declared` → first tighten the
  two `deny`-only hold texts named in the plan's Implementation review (the
  real repository and sha in the `unknown` hold; the content-id failure
  wording), as a greenbelay release; then run
  `greenbelay graduate push-browser-check --to deny`. Exit 0: record the
  figures in the rule's `Evidence` and the plan's Outcome, set the plan
  `status: done`. Exit 2 with `refusing to move` on stderr: the corpus does
  not carry it; stay at advise, record its lines, never `--force`. Any other
  non-zero exit (`could not record the stance`, a usage error): stay at
  advise, record its stderr, do not set done, never `--force`.
- >30% of advices `unreachable` → `greenbelay demote push-browser-check`:
  the rule rests at `observe` (journalled, silent, no questions asked), with
  the reason in the plan's Outcome.
- Otherwise stay at advise, record why, and set the next date.

## Evidence (counted 2026-11-28)

The journal keeps about 2.5 days: its oldest line is 2026-11-25 10:42 UTC.
Before that, only the weekly rollup (`~/.claude/greenbelay/rollup.log`) holds
the counts. It starts at 2026-11-21 08:37 UTC, after 6.6.0 was installed, and
it does not split advices by `check=`.

| window | passes (bound + remembered + declared) | advices | pass share |
|---|---|---|---|
| rollup, week of 11-16 (from 11-21) | 9 + 4 + 0 = 13 | 8 | 62% |
| rollup, week of 11-23 (to 11-25) | 25 + 27 + 0 = 52 | 30 | 63% |
| journal, 11-25 → 11-28 | 6 + 2 + 0 = 8 | 17 | 32% |
| all | 73 | 55 | **57%** |

- Advices by check, journal only: 14 `missing`, 3 `stale`, 0 `unreachable`,
  0 `gap`. The rollup's 38 advices have no split, so the unreachable share is
  0 of 17 measured. Demoting would need at least 17 of those 38 to be
  `unreachable`.
- Advised twice on one tree: the 17 journal advices fell on 9 trees, and
  three trees took 11 of them: `851f9014` (cluster-vision) 3 times,
  `d2d94c06` (happier, two sessions) 5 times, and `b8877b0c` (happier) 3
  times. Those pushes went out again without a check run.
- `remember-failed`: 0. `deny denied`: 0.
- `greenbelay corpus check`: `push-browser-check  17 reviewed (9 negative),
  0 unreviewed, precision 100%`.
- Neither bucket (not counted): 374 `no-ui-repository`, 154 `unresolvable`,
  38 `no-interface-change`, 1 `delete-only` (journal only).

## Resolution

Stay at advise. The pass share is 57%, under the 70% bar, and the latest
window falls to 32%. No advice is measured as `unreachable`, so this is not
a demote either. The misses are agents pushing again on an advised tree
without running the check, not checks that could not run. `graduate` was not
run, and the plan stays active. Next check: [t09](t09-push-browser-check-day-14.md),
due 2026-12-05.
