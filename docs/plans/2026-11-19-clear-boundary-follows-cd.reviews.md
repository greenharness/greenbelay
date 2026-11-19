# Reviews: clear-at-boundary follows a `cd`

## Full reviews (reference)

- **backend** (opus): round 1 approve-with-changes, 4 findings (redirections high; serde, 7-day check medium; pending_from low), all accepted; 48k, 100 s. Round 2 approve-with-changes: new high `dir=` from the wrong repository and export mismatch, low backtest bar; both applied. Re-run on the final body: approve, 61k, 84 s.
- **lang:rust** (opus): round 1 approve-with-changes, 5 findings (serde high, keyword prefix high, `2>&1` medium, unassigned medium, backtest medium); 3 and 4 duplicated unix's; 42k, 81 s. Round 2: the `dir=` export round trip (high), applied with backend's; 50k, 23 s.
- **tui** (opus): round 1 approve-with-changes, 4 findings (shape in the row, `dir=` hold row, tally, explain pilot), all accepted; 39k, 59 s. Round 2: `Kind::of` must strip `dir=` (merged with backend's), `tally` cannot skip a `watched` row (applied: the row counts); 44k, 22 s.
- **unix** (opus): round 1 approve-with-changes, 4 findings (unassigned high, redirect `&` high, `||`/zsh pipeline medium, non-literal words medium), all accepted; 39k, 85 s. Round 2: A low (`||` wording) and B medium (a relative `cd` word without `./`/`../` is Unknown, `CDPATH` is invisible) carried into implementation; C low (backtest gate) merged with backend's; 39k, 16 s.
- **codex** (gpt, high effort): rework. Applied: compound-scoped invalidation (the 4526105a `if … fi` case), walk from the entry cwd (a relative `cd` applied twice), conditional `&&`/`||` assignments, a correct backtest bar for the 994 calls. Carried: a separate due ticket for the 7-day measure, created at release (ADR-0077).
- **backend** re-run after codex: approve-with-changes, 67k, 45 s. Carried: strip a leading `then`/`do`/`else`/`elif`/`{` inside a compound before reading the command, with two table rows (medium); relabel the picture's "(payload)" (low).
- **Carried into implementation, in all:** unix A (`||` wording), unix B (relative `cd` word without `./`/`../` is Unknown), codex 5 (due ticket), backend 7 and 8, tui's round-2 low note.

## Delta 1 (2026-11-19, after the first backtest)

- **Why:** the first backtest gave 369/393 (93.9%), below the plan's bar. 19 of the 24 misses were `… && LIVE=~/x && …; git -C "$LIVE" worktree remove …`, made Unknown when the `&&` list ended and labelled `not-literal`. The backtest also caught a bug: `here` was compared with the walk's start, not the payload cwd. It is fixed in 08ba504b, and the correction is recorded against the relais run.
- **backend** (opus), delta: approve-with-changes, 53k, 98 s. Four findings, all accepted: the added-hold risk named, with a 7-day count (medium); `||` assignments stay Unknown (medium); look up the earlier binding before `vars` is written (low); state the rerun's expected count (low).
- **backend** re-run on the final body: approve-with-changes, 59k, 35 s. All four resolved. Carried into implementation: (5, medium) the 369/393 run is the one before Delta 1; (6, low) reword the post-release fallback, and make clear the `gh` example is a hold only when `gh` fails; (7, medium) the rerun must run on a build that implements Delta 1. Done in the Delta 1 commit: rerun 385/393 (98.0%), 796/796 kept; 6 of the 8 misses are exporter errors where the branch checks the right repository, and 2 are `cd-or`, journalled.
- `plan-apply` HTML-escaped `&&` in two patches. They were fixed by hand, so `plan-apply --check` reports those 4 as unverified (their `new_string` holds `&amp;`).
