# Full reviews: Every plan gets an id, and the panel counts

- **Round 1:**
  - backend (41k tokens, 76 s): the ID was retired too early (use PostToolUse `on_plan_answered`); by-title; idempotent writes; a live week check.
  - lang:rust (56k, 107 s): relais already rejects `seat_source`, `seat_classes` and `findings_unparsed`; retired at the pre-answer allow; by-title; a body→ID map; the write check; the `_` fields; the pipe test; a urandom fallback.
  - architect (29k, 25 s): retired too early; by-title and by-body; a schema token for approvals; the data-names rebase.
  - unix (33k, 52 s): plan-apply's write protocol; the failure exit; the pipe test.
  - tui (39k, 74 s): the 📍 drop order; `releases` without CHANGELOG; the count words; messages and a stdout test (its bold Patches block was applied by hand).
- **Round 2:**
  - backend and lang:rust found one high together: write the approval before the lock (applied, with unix's non-fatal-hook wording);
  - architect approved;
  - tui and unix: approve-with-changes, with mediums.
- **Final body:** all five approve.
- `plan-panel` printed `panel=delta` throughout: the bug this plan fixes. All five seats were chosen by hand.
- An earlier heuristic version (branch/H1 chains) was reviewed and superseded by your choice; it is kept at /tmp/b9-heuristic-plan.md.
