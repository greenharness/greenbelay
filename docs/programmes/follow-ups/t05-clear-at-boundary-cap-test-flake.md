+++
type = "task"
mode = "afk"
status = "open"
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
