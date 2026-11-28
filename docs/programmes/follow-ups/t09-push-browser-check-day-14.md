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
- Count from the extract, not the journal. The journal rotates in about 2.5
  days, and the rollup drops `check=`. Since 2026-11-28, the launchd agent
  `com.fredericrous.t09-pbc-extract` has run
  `~/.local/state/t09-pbc-extract/extract.sh` at 02:10 and 14:10. It writes
  the rule's lines, `LC_ALL=C sort -u`, into
  `~/.local/state/t09-pbc-extract/push-browser-check.lines`. It skips the live
  journal's last line, which may be half-written. It was seeded from the t22
  snapshots, so it holds every line from 2026-11-23 02:30 UTC.
  - Check its `launchd.log` for a run each half-day, and that the first and
    last stamps cover the window.
  - Lines carry second-resolution stamps, so two identical lines in one
    second collapse into one. That undercounts by at most one push.
  - After counting: `launchctl bootout gui/$(id -u)/com.fredericrous.t09-pbc-extract`,
    then delete its plist. Keep the extract until the rule leaves advise.
  - If the rule stays at advise again, file making the rollup keep `check=`
    for advise rows, so that the next check needs no agent.
- For each tree advised more than once: read that session's transcript at
  the second push. Was the advice shown, and did the agent read it as
  optional? If so, the advice text is the thing to fix.

## Done when

The answer is recorded as in t01, with an unreachable share measured over
the whole window from 2026-11-23, and the extract agent removed.
