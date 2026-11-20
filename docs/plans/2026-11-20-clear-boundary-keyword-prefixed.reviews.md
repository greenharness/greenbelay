# Reviews: clear-at-boundary sees a boundary whose first word is a keyword

**Round 1** (opus, all four):
- backend, approve-with-changes (42k, 84 s). [high] Low 2 had to keep `not-literal` before eval/source/. — applied. [medium] count and read the rows Low 2 moves — applied. [medium] heredoc body row and test — applied. [low] cap in the Picture — applied.
- lang:rust, approve-with-changes (41k, 64 s). [medium] heredoc — rejected, duplicate of backend#3. [low] a test that loops over the const — applied. [low] two-boundaries cap test — applied. [low] `command -v cd` row — applied.
- tui, approve-with-changes (43k, 73 s). [medium] read the cap pilot by row body, since the `explain` tally counts both rows as `watched` — applied.
- unix, approve-with-changes (37k, 53 s). [medium] name the option words, `--` included — applied. [low] the const holds the 8 openers, and the closers keep their own arms — applied. [low] the decision log notes `command git` and `if ! git` — applied.

**Round 2** (resumed):
- backend, approve-with-changes (48k, 17 s). All 4 resolved. Carried: [low] the `command -- cd` row belongs in the Behaviour table; [low] fill the Review panel (done here); [low] add a third t23 row for `command`/`builtin`.
- lang:rust, approve-with-changes (45k, 15 s). All 4 resolved. Carried: [low] the `command -- cd` row, the same as backend's.
- tui, approve (47k, 17 s). Resolved. Carried: [low] the `command -- cd` row, the same.
- unix, approve-with-changes (39k, 15 s). All 3 resolved. Carried: [low] tidy Low 2's wording; [low] the heredoc test asserts the same dirs as the bare-line case.

**Carried into implementation:** a `command -- cd /h/b && git worktree remove x` → `not-literal` row and its test; the tidied Low 2 wording; a third t23 table row; the heredoc test checks the bare line's dirs.

