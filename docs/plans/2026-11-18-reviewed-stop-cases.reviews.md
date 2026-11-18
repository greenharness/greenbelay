# Full reviews: Reviewed Stop cases, then clear-at-boundary to deny

Round 1 (body 193f5109):

- **backend, approve-with-changes** (51k tokens, 81 s), 7 findings, all
  accepted:
  - [high] `pending-work` needs a frozen `state.pending`;
  - [high] `pending background` negatives are circular;
  - [medium] the command negatives are vacuous;
  - [medium] fixture volume;
  - [medium] replay-agreement check;
  - [low] journal wording;
  - [low] no phase merge.
- **tui, approve-with-changes** (41k, 74 s), 3 findings, all accepted:
  - [high] a disagreement names its `why` and the fixture path;
  - [medium] stdout and stderr channels, exit 2 outside a checkout;
  - [medium] an idempotent export.
- **unix, approve-with-changes** (34k, 72 s), 4 findings, all accepted:
  - [high] build in memory, write temp then rename, print the `?` lines
    last;
  - [medium] `--out DIR`;
  - [medium] skipped Stops and exit codes;
  - [medium] `GIT_CONFIG_GLOBAL` and `XDG_CONFIG_HOME` in the scratch run.
- **lang:rust, approve-with-changes** (61k, 152 s), 6 findings, all
  accepted:
  - [high] a `persist` closure keeps the `state-unwritable` order;
  - [high] `state.pending` frozen (merged with backend's finding);
  - [medium] the last tail entry's uuid, not `prompt_uuid`;
  - [medium] `not_clean` frozen;
  - [medium] a 64 KiB cap;
  - [low] the uuid survives the 200-character trim.

Round 2 (body 55cf02c8). Every round-1 finding is resolved. These are
carried into implementation:

- **backend, approve-with-changes** (57k, 20 s):
  - [medium] the Goal says 8 labelled boundaries, Behaviour says 12. 12
    holds.
  - [medium] exit 1 on a re-run where everything is already exported. Exit
    0 when nothing failed.
  - [low] name the failing-`persist` and uuid-trim tests in Phase 1.
  - [low] replay compares "fires at `deny`" with "journal says advised or
    denied".
- **tui, approve-with-changes** (47k, 16 s): [medium] the same exit-code
  point. The summary says `nothing new: N already exported`.
- **unix, approve-with-changes** (39k, 16 s): [low] the same exit-code
  point.
- **lang:rust, approve** (71k, 38 s): [low] print the stderr summary after
  the writes and before the case lines, because SIGPIPE is `SIG_DFL`.

Codex final review (gpt, high effort; body 55cf02c8):
approve-with-changes. All 3 findings were high and accepted, and applied
verbatim:

- trimming keeps the structural fields the judge reads, and the original
  and redacted windows must replay the same;
- the hook's byte window is rebuilt before trimming;
- labels are read against the original transcript, with a rationale and a
  locator.

Codex delta (body f5e6add4). All four seats re-ran, and none raised a new
blocker. Carried into implementation:

- **backend, approve** (66k, 39 s):
  - [medium] an excluded fixture never enters the embedded tree, and stays
    `?` under `# excluded: <why>`;
  - [low] the rationale goes in a `# why: … (<session8>, <uuid>)` comment;
  - the round-2 items are still open: the Goal's 8 against 12, the exit
    code, the Phase 1 test list, and the comparison at `deny`.
- **lang:rust, approve** (75k, 33 s): [medium] add one reader,
  `read_tail_until_at(path, end, …)`, which the live
  `read_tail_until(end = size)` calls, so the window logic has no second
  copy.
- **tui, approve-with-changes** (53k, 27 s):
  - [medium] an excluded fixture is printed as `# excluded: Stop <name>:
    <why>` with no case line, and the summary counts it;
  - [low] the `# why:` line goes above the label.
- **unix, approve** (44k, 21 s): [medium] add `"countable": false`, so
  `judge_case` returns `Err("not countable")` and a label can never count
  the case.
