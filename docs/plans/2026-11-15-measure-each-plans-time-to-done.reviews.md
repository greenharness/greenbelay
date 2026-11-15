# Measure each plan's time to done: full reviews

- Round 1, all approve-with-changes, 24 findings applied:
  - backend (36k tokens, 51 s): adds=contract; `report` imports usage by default; worktree map keyed by time; pre-worktree time; `unreviewed` count; unmatched relais hours; runtime.
  - lang:rust (41k, 60 s): merge rows carry no `first_round_sha`; clap read-only; attribution fixtures; clock-free test; civil dates; flag combinations.
  - tui (28k, 30 s): exit codes; 80 columns; TTY progress; tool detection; `--since` semantics; emoji header.
  - unix (35k, 49 s): read-only; array shape and default `--since`; exit codes; `/bin/date`; `RETRO_DIR`.
- The main session measured the start key: the landed sha resolves 17 of 157; the machine comment resolves 75 of the 90 that carry one.
- Round 2:
  - backend (47k, 22 s): the high finding (`unreviewed` out of medians) applied, the rest carried.
  - tui (34k, 12 s): 4 carried.
  - unix (42k, 20 s): 1 carried.
  - lang:rust (50k, 27 s): 2 carried.
- New seats:
  - architect (35k, 51 s): all 4 applied (`schema` token, subcommand, overlap note).
  - po (28k, 25 s): 2 high rejected (Outcome is filled at close; parallel build is the person's ask), 1 medium rejected (t01 names PRs and releases), coverage line and old-target record applied.
- backend on the final body (55k, 19 s): approve, no blocker.

