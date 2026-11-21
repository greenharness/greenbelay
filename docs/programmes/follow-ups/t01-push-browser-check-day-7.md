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
  on or after 2026-11-21: `unconfirmed bound` and `unconfirmed declared` are
  passes; `advise advised … check=missing|stale|unreachable|gap|unknown` are
  advices; `unconfirmed delete-only|dry-run|no-ui-repository|…` and the
  `deny denied` lines of a refused store write are neither and stay out of
  the counts.
- Among the advices, how many carry `check=unreachable`, and how many pushes
  were advised twice on the same tree (an advise the agent could not
  satisfy).
- `greenbelay graduate push-browser-check --to deny` as a dry read of its
  corpus lines (17 cases, 9 `nomatch`, above the 12/4 floor).

## Answer looks like

- ≥70% of UI pushes `bound` or `declared` → run
  `greenbelay graduate push-browser-check --to deny`, record the figures in
  the rule's `Evidence` and the plan's Outcome, set the plan `status: done`.
- >30% of advices `unreachable` → `greenbelay demote push-browser-check`,
  with the reason in the plan's Outcome.
- Otherwise stay at advise, record why, and set the next date.
- Before deny, tighten the two `deny`-only hold texts named in the plan's
  Implementation review (real repository and sha in the `unknown` hold; the
  content-id failure wording).
