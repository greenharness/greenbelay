# Full reviews: keep weekly firing counts past journal rotation

## Full reviews (reference)

Round 1 (backend, rust, tui, unix): all approve-with-changes; 23 findings, 22 accepted, 1 rejected (a failed fold stays a miscount by the module rule). Round 1 plus architect and po: approve-with-changes.

Carried into implementation (round 2 medium and low, no body edit):
- Release 1 must keep Phase 3's `C` rows when it merges: carry unknown line kinds through unchanged, and test it (rust, medium). Phases 1–2 and 3–4 are two PRs, each with its own release (rust, medium).
- Test the busy-lock skip: generations stay, the next append folds (backend, rust). Bound the skip: a journal over twice its cap rotates anyway and writes `rollup-failed lock-busy` (architect, po).
- `--json`: `replay` is `exact|confirm|live|none`; `tool` is an open string; add `lost_folds` and `L` rows so a lost fold stays visible (backend, architect).
- Freeze window: 7 days is in the body, po asks for 21 (transcripts here reach back 39 days), and backend cites `cleanupPeriodDays` 30. Decide at Phase 3 from the live transcript range. Retro's weekly run is the caller that freezes each week (po, backend).
- Text output: mark a partial week with a trailing `*` and keep the week cell 11 columns; list `rollup [--json] [--weeks N] [--transcripts <dir>] [-q|--quiet]`; keep warnings under `-q`; a missing state dir is an empty roll-up (tui, unix).
- EPIPE exits 0 through `writeln!` on locked stdout; test `rollup | head` (rust, unix). Verify the SIGPIPE site in `src/main.rs` in Phase 3.
- Pilot against a synthetic 104-week `rollup.log` and time the fsync alone; check two weeks after release that `rollup.log` is under 50 KB and `rollup-failed` is under 1% of rotations, rolling back above 5% (backend, architect, po).
- Retro keeps `uncounted` when `rollup` is missing, and F3 reports how many of the 50 rules map to `None` (architect, po). Draw the transcript walker in the Picture (backend).
