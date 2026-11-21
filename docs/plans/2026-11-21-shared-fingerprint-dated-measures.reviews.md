# Reviews: A shared fingerprint for dated measures

## Full reviews (reference)

Carried into implementation (medium/low after round 2):
- backend#9: the baseline records its own first run (`runs` holds it), so two runs give 2.
- backend#13 / python: a late re-stamp checks the pinned `count-context.py` blob still matches before stamping, else the error goes to `.err`.
- backend#12: a Verification row for an unpaired day (delete a stamp, then a `.txt`: exit 1 each, `daily.py` repairs, then exit 0).
- rust#7: the read-rate fixture test passes `--root` to a temp dir, never `~/.claude/projects`.
- rust#8: `Refused` carries keys, values and the path; the CLI caller appends the accept command.
- rust#9: the Rust compare test builds stamps through the helper's `stamp()` (no sha2, no hard-coded hashes), and a hand-edited recipe gives exit 1.
- rust#10: `<sidecar>.lock` stays; the self-test checks only that no `mkstemp` temp is left.
- tui#6: the incomplete-day message names `daily.py`, not `--rebaseline`; corruption message says "stored fingerprint does not match its recipe".
- tui (Codex delta): `--sum` prints a null `at` as `counted 2026-11-19`; prints `capture running, waiting` before blocking on `t09.lock` (unix agrees: try `LOCK_NB` first).
- tui#4 (r3): `--sum` matches `YYYY-MM-DD.txt` / `.fp.json` only; the self-test has a stray `.txt` that is ignored.
- python: a stamp with `at: null` is valid (not a missing key) and is in the self-test; `fcntl` is Unix-only, said in the docstring; the "8 in 15,544" figure is recomputed from the `.txt` files.
- tui: for `compare` and `--sum`, the accept command is their own argv plus `--rebaseline`.

Round 1 (all approve-with-changes): rust 6 findings (26k, 21 s); python 6 (33k, 44 s); backend 7 (36k, 53 s); tui 5 (28k, 41 s); unix 4 (29k, 40 s). Round-1 body not saved in the ledger (recorded after the first edit).
Round 2: python approve; tui approve-with-changes (2 carried); backend approve-with-changes (#8 later folded into body, #9 carried); rust approve (2 low carried); unix approve-with-changes (#5 folded into body).
Codex (gpt, high effort): approve-with-changes, 3 high + 1 medium, all applied (no `counted` line in 11-14..18 `.err`; digest verification; capture/sum lock and pairing; read-rate week is in the timestamp's own offset).
Codex delta: python approve, tui approve, rust approve-with-changes (#9 carried), backend approve-with-changes (#10, #11), unix blocking #6 (flock self-deadlock) + high #7 (`--sum` dropped an unstamped `.txt`); applied as 3 patches.
Final delta: unix approve, backend approve-with-changes (2 low carried), python approve, tui approve, rust approve.

