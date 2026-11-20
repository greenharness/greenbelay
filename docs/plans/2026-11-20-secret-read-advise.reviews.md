# Full reviews: secret-read

## Full reviews (reference)

- backend (opus): round 1 approve-with-changes (once-memory missing on Bash; `on_grep` early returns; `sed -i` and `whole-file-dump` at 8 KB; no backtest bar; files unnamed). Round 2 new blocker, journal keeps a typed secret on repeat calls: fixed. Carried: shared `secret_file` across the three tools; glob expansion before the exists check; say "unconfirmed" not `Confirmed::No`; one told-once key across Bash, Read and Grep.
- lang:rust (opus): round 1 blocking, no once-memory on Bash, and `session_state::record` is the wrong store; `on_grep` early return; operands per program. Round 2 new blocker, the gate cannot live in `confirm` (crate boundary, no agent id, `greenbelay check` would burn it): fixed, moved to `on_bash`.
- tui (opus): round 1 Bash once-memory, Grep `output_mode`, advice text, journal excerpt. Round 2 resolved; carried: reason text in future tense, true for `cp`.
- unix (opus): round 1 `$HOME` and quoted words, `<` redirects and pattern operands, quiet-flag clusters, indirect reads, `.pem`/`.key` breadth: all resolved. Round 2 carried: shell-quote `{path}` in remedies, `.envrc` gets the generic remedy, do not mark a path told when the finding is later suppressed.
