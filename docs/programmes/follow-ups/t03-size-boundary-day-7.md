+++
type = "task"
mode = "afk"
status = "open"
blocked_by = []
repos = ["greenbelay"]
due = "2026-11-30"
+++
# Does the size boundary keep its stance, seven days after the release

## Question

The size kind of `clear-at-boundary` (ADR-0083, plan
`docs/plans/2026-11-23-hand-off-mid-phase-400k.md`) hands off mid-phase past
400k context. It ships at `observe` (the 30-day replay: 2.71 fires a
day, 15.5% while a task launched outside the transcript still ran, against a
bar of 10%). Seven days after the release, does the live observe evidence
graduate it to `deny`?

## Counts

- `greenbelay size-evidence --days 7`: the size fires by outcome, the
  handoff requests, how many a clear-relay `submit` answered, how many were
  unheeded (a session with no relay log included: the relay logs only once
  it arms), and how many are missing evidence (the relay directory or a log
  could not be read). Check the `relay logs read from` line names the
  plugin's `stateDir`. The report is read by a person; nothing parses it, so
  it has no `--json` until something does.
- Heeded: of the `size fired` journal rows still in the journal, how many
  turns ended with `safe to /clear`.
- Rerun `~/.local/state/sess-audit/tokscan/size400.py`: the share of Opus
  spend after the first idle turn end past 400k, against the baseline of
  61.0% (30 days to 2026-11-23, $32.0k over 314 sessions).

- At `observe` the rows are `observed`: for each, read whether a background
  task, a relais run or a question was in flight at that Stop (the live
  payload's `background_tasks` is what the replay could not see).

## Bar

- Graduate (`git config --global greenbelay.clear-at-boundary.size-stance
  deny`, then the compiled default in a release) when the observed fires
  are at least 1 a day and at most 10% of them had something in flight.
- After `deny`, the target: the share of spend past the first idle turn end
  past 400k falls to at most 50%. Roll back to `observe` if fewer than 50% of
  handoff requests are followed by a clear-relay `submit`, if Opus
  list-price spend per active session-day rises against the baseline, or if
  more than 1 of 5 resumed sessions, read by hand, redid work the handoff
  named as done.
