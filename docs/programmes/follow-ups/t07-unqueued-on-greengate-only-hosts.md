+++
type = "task"
mode = "afk"
status = "open"
blocked_by = []
repos = ["greenbelay"]
+++
# `heavy-build-unqueued` on a host with greengate and no amont

## Question

`heavy-build-unqueued` confirms only when `amont --version` on `PATH` is
1.62.0 or newer, so a host with greengate 5.0 and no `amont` alias never
hears the rule, `slot run --now` included. Should `confirm` accept either gate
(`greengate` at 4.0 or newer as well), and should the bare-command advice
name `greengate slot run` when that is the gate found?

## Also

- The advice echoes at most `ECHOED_WORDS` (8) words of the command, so a
  longer command's queued form drops arguments silently. Mark the cut with
  `…`, or echo the clause from its span.
- If greengate is rolled back to 4.13 (host-slot-admission t17), the
  bare-command advice names `--now`, which 4.13 lacks: gate that sentence on
  the gate version found.

## Look at

- `crates/greenbelay-guard/src/rules/heavy_build_unqueued.rs`: `confirm`,
  `amont_has_slots`, `ECHOED_WORDS`, the `Launch::Bare` remedy.
- Plan `docs/plans/2026-11-25-now-is-unqueued.md` (Phase 6 of amont's
  host-slot-admission): the backend reviewer's round-3 findings, ruled
  outside that slice.

## Done when

A test with only `greengate` on `PATH` shows the rule confirming (or the
ticket records why it stays amont-only), and a nine-word command's advice
does not drop arguments unmarked.
