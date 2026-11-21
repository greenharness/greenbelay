# push-browser-check: full reviews (reference)

Panel record of [2026-11-20-push-browser-check.md](2026-11-20-push-browser-check.md).


Carried into implementation (not edited into the body):
- rust r2#7 (medium): per-case text is `confirm`'s `Confirmed::YesSaying` reason/remedy; `examine` keeps a fixed one-line finding; test a `git -C` push names the exact block.
- tui r2#1 (medium): print the TREE block alone on its own line (94 cols for greenrack). tui r2#2 (low): write out the Gap sentence.
- backend r2#1 (medium): add `Need::BrowserCheck` to `crates/greenbelay-process/src/agents.rs` (`Need`, `matches`, `gap_line`). r2#2 (medium): test a report with `browser:` but no `url:` → advise. r2#3 (low): draw Gap and Overrule in the Picture.
- architect #2 (medium): add `Need::BrowserCheck` to `review_gaps` so `greenbelay doctor` lists it. #3 (medium): a greenharness-skills test pins the `browser:`/`url:`/`unreachable:` lines of `browser-check.md`. #4 (low): store guards in `implementation_review.rs` are becoming a registry; move later.
- po #3 (medium): the 7-day window starts at the skills install. #4 (low): count browser-check re-runs per UI push.
- backend r3#1/#2 (medium): state how the fork-only and 30-day counts are taken (transcript scan of `Agent` launches).
- backend delta #2 / rust delta #9–#10 (low): time a push in a session with ≥20 side transcripts; read main first, stop at the first bound check; unit test that `side_transcripts` never lists another session's dir; `side_files` returns the same paths and mtimes after the refactor.

Rounds:
- Round 1 — backend approve-with-changes (37k, 44 s): adds=contract, fork false advise, verification checks, drop `tree:` echo, file names, skills profile. tui approve-with-changes (38k, 44 s): per-case reason, hook-printed block, gap when agent missing, marked question. unix approve-with-changes (36k, 45 s): `browser:`+`url:` lines and `unreachable:` one-liner, `-C`/rev, atomic write; sweep 40-hex rejected. lang:rust approve-with-changes (51k, 75 s): `Judged::Gap` via `agents::carried`, backtester never runs `confirm`, no by-patch carry-over, own small store + `HOOK_ONLY_DIRS`, `crate::content_id`.
- Round 2 — unix approve; rust, tui, backend approve-with-changes (mediums carried); architect approve-with-changes (39k, 46 s; high: store refusal names its own remedy); po approve-with-changes (31k, 46 s; high: baseline count and stop rule, success/rollback numbers). Backend re-run: approve.
- Delta (person: read fork transcripts) — backend high: derive the side dir from this session's `transcript_path`; rust: new `plan_review::side_transcripts`. Backend re-run caught a plan-apply HTML-escape on the signature (fixed by hand); backend: approve.
- Delta (person: full automation, no human in the loop) — Overrule/Check now/Hold question removed; `## Browser check` → `none: <reason>` escape read like push-preview's `evidence:`; day-7 check runs `graduate --to deny` or `demote`; rollback signal = share of `unreachable:` advises. Backend (72k, 35 s) high: `--to deny` and the 12/4 corpus floor; medium: generalise `evidence_of` over (section, prefix) with a byte-identical journal test; low (carried): plan approval stays the person's step, before the push. Final backend (74k, 67 s): approve.
