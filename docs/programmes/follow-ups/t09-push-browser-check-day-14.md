+++
type = "task"
mode = "afk"
status = "open"
blocked_by = []
repos = ["greenbelay"]
due = "2026-12-05"
+++
# push-browser-check, day 14: does the pass share reach 70%

## Question

On 2026-11-28 ([t01](t01-push-browser-check-day-7.md)), UI pushes passed 57%
of the time (32% in the last 2.5 days). Most advices were repeats on a tree
the agent pushed again before checking; two of the three repeated trees
never got a check. Does the share reach the bar by
2026-12-05? If not, why does an advised session push again instead of
running the check?

## Look at

- Count it the same way as t01: same outcome buckets, same ratio, same
  answer order (≥70% → hold texts, then graduate; >30% `unreachable` →
  demote; otherwise stay and record why).
- The journal rotates in about 2.5 days, and the rollup drops `check=`.
  Before counting, either:
  - extract this rule's journal lines into a file outside the
    `journal.log*` glob every day or two, de-duplicated (the rotated files
    overlap from one run to the next):
    `grep -h ' push-browser-check ' ~/.claude/greenbelay/journal.log* | sort -u -o <extract> - <extract>`
    (`touch <extract>` first; the rotated files are plain text), or
  - make the rollup keep `check=` for advise rows,

  so that the unreachable share covers the whole window, not the last 2.5
  days only.
- For each tree advised more than once: read that session's transcript at
  the second push. Was the advice shown, and did the agent read it as
  optional? If so, the advice text is the thing to fix.

## Done when

The answer is recorded as in t01, with an unreachable share measured over
the whole window.
