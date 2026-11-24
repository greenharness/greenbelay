+++
type = "task"
mode = "afk"
status = "closed"
blocked_by = []
repos = ["greenbelay"]
+++
# The cap test of clear-at-boundary flakes on a jammed host

## Question

`tests/clear_at_boundary.rs`
`when_the_cap_silences_a_plan_only_boundary_plan_phases_open_runs` failed in
a pre-commit run on a jammed host on 2026-11-21: the journal lines straddled
two seconds and no `watched ... cap` entry was found. It passed 3 of 3 alone
the same day. What makes the test depend on the clock, and how does it stop?

## Evidence

- Inbox line 4bca21ec, 2026-11-21.
- The size-boundary plan (2026-11-23) edits the same file; expect the flake
  on a jammed host until this is fixed.

## Resolution

The clock is the Stop's shared 2-second deadline
(`clear_boundary::DEADLINE`): every git child of one Stop shares it. On a
jammed host the fourth Stop's `git status` ran past it, so the rule
journalled `watched timeout` before it reached the cap check, and
`plan-phases-open` spoke in its place. Only the last assertion failed.
Reproduced with a fake git that sleeps 2.5 s on `status`.

Fixed by `GREENBELAY_STOP_DEADLINE_MS`, a test override written like
`GREENBELAY_GH_DEADLINE_MS`. The `clear_at_boundary` and `plan_phases_open`
tests pin it at 60 s. The timeout test passes 2 s explicitly. The new
`a_slow_git_still_reaches_the_cap_under_the_test_deadline` holds the cap
with git slower than the shipped deadline. The shipped deadline is unchanged.
