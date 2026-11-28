# Full reviews: greenbelay drops the retired variable notices

## Round 1 (body 5d8aec283207)
- **backend** (opus, 36k tok, 38 s): approve-with-changes. 1 [medium] `/nonexistent/aval` cannot prove "not spawned": use a marker fake; 2 [medium] no check for "byte-for-byte 6.18.2"; 3 [medium] census on origin/main before the merge, no expected list; 4 [low] ADR list; 5 [low] merge Phases 1 and 2. All accepted.
- **lang:rust** (opus, 32k, 29 s): approve-with-changes. 1 [medium] marker executable; 2 [medium] assert neither name and set == unset; 3 [low] name the tests, `--exact`; 4 [low] drop the empty-value branch. All accepted.
- **unix** (opus, 28k, 23 s): approve-with-changes. 1 [medium] marker + stdout/exit equality in both modes; 2 [medium] census on HEAD then origin/main; 3 [low] pilot diffs stdout. All accepted.
- **tui** (opus, 30k, 28 s): approve-with-changes. 1 [medium] check the names, not the phrase; 2 [low] cmp against 6.18.2; 3 [low] `-i` makes `[a-z]Aval` hit inside words. All accepted (regex split).

## Round 2 (body c6f652667256)
- **lang:rust** (36k, 12 s): approve; all resolved.
- **unix** (31k, 11 s): approve; all resolved.
- **tui** (34k, 10 s): approve; all resolved.
- **backend** (45k, 29 s): approve-with-changes; 1–5 resolved; new 6 [medium] the census list misses the vendored `.adr/packs/decisions.pack` (history). Carried into implementation.

## Codex (gpt, high effort)
- approve-with-changes. 1 [medium] the tests set `GREENBELAY_GREENSPOTTER`, which would mask a fallback to the retired variable when the replacement is unset: run the set cases with the replacement overrides removed and PATH fakes. 2 [medium] same as backend 6. Both carried into implementation.
