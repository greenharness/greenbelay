# Reviews: clear-at-boundary follows a `cd`

## Full reviews (reference)

- **backend** (opus): round 1 approve-with-changes, 4 findings (redirections high; serde, 7-day check medium; pending_from low), all accepted; 48k, 100 s. Round 2 approve-with-changes: new high `dir=` from the wrong repository and export mismatch, low backtest bar; both applied. Re-run on the final body: approve, 61k, 84 s.
- **lang:rust** (opus): round 1 approve-with-changes, 5 findings (serde high, keyword prefix high, `2>&1` medium, unassigned medium, backtest medium); 3 and 4 duplicated unix's; 42k, 81 s. Round 2: the `dir=` export round trip (high), applied with backend's; 50k, 23 s.
- **tui** (opus): round 1 approve-with-changes, 4 findings (shape in the row, `dir=` hold row, tally, explain pilot), all accepted; 39k, 59 s. Round 2: `Kind::of` must strip `dir=` (merged with backend's), `tally` cannot skip a `watched` row (applied: the row counts); 44k, 22 s.
- **unix** (opus): round 1 approve-with-changes, 4 findings (unassigned high, redirect `&` high, `||`/zsh pipeline medium, non-literal words medium), all accepted; 39k, 85 s. Round 2: A low (`||` wording) and B medium (a relative `cd` word without `./`/`../` is Unknown, `CDPATH` is invisible) carried into implementation; C low (backtest gate) merged with backend's; 39k, 16 s.
- **codex** (gpt, high effort): rework. Applied: compound-scoped invalidation (the 4526105a `if … fi` case), walk from the entry cwd (a relative `cd` applied twice), conditional `&&`/`||` assignments, a correct backtest bar for the 994 calls. Carried: a separate due ticket for the 7-day measure, created at release (ADR-0077).
- **backend** re-run after codex: approve-with-changes, 67k, 45 s. Carried: strip a leading `then`/`do`/`else`/`elif`/`{` inside a compound before reading the command, with two table rows (medium); relabel the picture's "(payload)" (low).
- **Carried into implementation, in all:** unix A (`||` wording), unix B (relative `cd` word without `./`/`../` is Unknown), codex 5 (due ticket), backend 7 and 8, tui's round-2 low note.
