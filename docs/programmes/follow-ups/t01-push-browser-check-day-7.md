+++
type = "task"
mode = "afk"
status = "open"
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
