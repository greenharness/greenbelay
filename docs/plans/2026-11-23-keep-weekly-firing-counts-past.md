---
status: active
plan-id: p-01M83MQMJ5N7RZ01HHEZ6ZERTX
branch: feat/rollup-fold
repos: [greenbelay, greenharness-skills, blue-factory]
adrs: []
---
# Keep weekly firing counts past journal rotation

## Review panel

👉 **Decide:** none — approve if the bet is a weekly roll-up folded at rotation, with its carried findings, is the right fix.
📍 greenbelay, greenharness-skills, blue-factory · 6 phases · 2 releases · 3 PRs · next: Phase 1, the label rename. Panel: backend, rust, unix, tui, architect, po (all opus).
**Changed by review:** a busy roll-up lock skips the rotation; t34 stays open until retro reads `rollup`; the fold ships first as its own release.
**Verdicts:** round 2: 1 approve, 3 approve-with-changes; 22 findings carried, listed below; words 1459 → 1930 (×1.32); remove 0, merge 0.
📄 Full reviews: [2026-11-23-keep-weekly-firing-counts-past.reviews.md](2026-11-23-keep-weekly-firing-counts-past.reviews.md)

## Picture

```
 today                         after this plan
 -----                         ---------------
 journal.log .. .10            journal.log .. .9  --rotate--> .10
   .10 deleted by rotate                            |
   (firings lost)                          fold into rollup.log
                                           (week rule stance outcome arm n)
 status: "last 30 days"                                |
   reads 2 days            rollup --> week rule firings calls rate
```

Rotation used to delete the oldest journal generation and its firings. After this plan it folds them into a weekly roll-up first, so counts outlive the journal.

## Context

A backtest replay counts 12 of 62 greenbelay rules. The other 50 can only be counted from the journal, which keeps about 2.7 days (256 KB x 10 generations). So `status`'s `seen, last 30 days` column really reads 2 days, and retro's Target line writes `uncounted` for a rule whose `replay` is `none`. t29 decided a weekly roll-up, not a longer journal; this is its build (blue-factory ticket t34). Measured cost: about 10 KB a week.

## Goal

`journal::rotate` folds the generation it is about to delete into `rollup.log`, beside the journal. `greenbelay rollup [--json]` prints `week rule firings calls rate` for every rule. `status` reads roll-up plus journal and labels its column truthfully.

## Non-goals

- A longer journal (t29 rejected it).
- Back-filling weeks before the release. The roll-up counts from its first fold; nothing earlier comes back.
- Any scan of transcripts on the hook path. Only `greenbelay rollup` reads them.
- Changing `backtest`'s Bash-only weekly buckets. The new per-tool scan sits beside them.
- Counting `rotate-failed` or `rollup-failed` records in the roll-up. The two F records stay in the journal.

## Code review
- greenbelay: backend, lang:rust, unix (tui left out: one plain-text table, no interactive screen)
- blue-factory: none — Phase 6 edits the map and handoff text only
- greenharness-skills: none — its Phase 6 is a one-line skill edit made after the release, in a separate plan and PR

## Decisions made in this plan

1. **One week function.** `civil::week_start`, UTC Monday-start, the same one `backtest` uses. The fold, the calls freeze, `tally` and the status window all call it. Labels print via `civil::iso`.
2. **Coverage start.** `rollup.log` carries `S <ts>`, written once by the first fold: the oldest record that fold saw. The `status` label becomes `seen, last 4 weeks + this week` only when `S` is at or before the start of the oldest of the four closed weeks. Until then it reads `seen since <date>`, from `oldest_record()` (or `S`, whichever is older). A partial first week never passes as a full one.
3. **Rule to tool.** Rules declare no tool, and the journal record carries none. `rollup.rs` adds one table `rule_tool(id) -> Tool` with `Bash`, `Named("Read")`, `AnyTool` (sum of all tools) or `None`.
   - Every rule with replay `exact`, `confirm` or `live` maps to `Bash`.
   - The `NOT_SHELL` rules map explicitly, one by one.
   - A rule with no honest tool (SessionStart or Stop events, assertions) maps to `None`: `calls` and `rate` print `-` in text and `null` in JSON. They never divide by an unrelated count. `rate` is `-`/`null` also when `calls` is 0, so the JSON never carries `inf` or `NaN`.
   - A test fails when any rule id in the rule table has no entry, so a new rule cannot ship unmapped.
4. **Calls population.** A new walker counts every tool name per UTC week from the transcripts, **sidechain files included** (hooks fire inside subagents). It uses `transcript::for_each_transcript`, deduplicated on `tool_use.id`, bucketed on the entry's own `timestamp`. It is not a `backtest` reuse, because that scan is fixed to one tool name. A test pins that a `<session>/subagents/*.jsonl` call is counted.
5. **Write path.**
   - `rollup.log` is rewritten whole: read, merge, write `rollup.log.new.<pid>.<n>` in the same directory, fsync, rename (a locked writer removes leftover temps older than 60 s). It is never appended, so a crash leaves the old file or the new one, never a torn line.
   - The format header is `greenbelay-rollup-v1`. A reader that meets an unknown version skips the file (counts nothing, never blocks), per the journal module rule; `rollup` and `status` say so on stderr (`rollup.log has format <v>, skipped; upgrade greenbelay`).
   - The fold holds the existing rotation lock (`journal.log.0`) and then `rollup.log.lock`. `greenbelay rollup` takes only `rollup.log.lock`. The lock order is fixed, so the two cannot deadlock.
   - A stale lock (older than 60 s) is removed, as the rotation lock's is. So no holder keeps a lock near 60 s: `greenbelay rollup` scans transcripts holding no lock, then takes `rollup.log.lock` only to re-read, merge and rename. The fold tries the lock once and never waits; when busy it skips this rotation (the journal stays one record over its cap and the next `append` retries, as a busy rotation lock does), so no generation is deleted unfolded. Only a fold that fails on I/O writes `F … rollup-failed <why>` and rotates anyway. A test runs a fold during a `rollup` run and both sets of rows survive.
6. **Exactly once.** The roll-up carries a watermark `W <max ts folded>`. A fold skips rows at or below it. A crash between the roll-up rename and the journal rename re-folds the same generation, and the watermark makes that a no-op. Records that share the watermark's second may undercount by one: it is a count, so it may miscount and never block.
7. **Lines.** A closed `week` is the Monday's `YYYY-MM-DD`.
   - `S <ts>`
   - `W <ts>`
   - `R <week> <rule> <stance> <outcome> <arm> <count>`
   - `C <week> <tool> <count>`

   `arm` stays in the key so `tally` drops `arm=shadow`.
8. **Frozen calls.** `greenbelay rollup` writes `C` rows only for closed weeks, the first time they are computed. A week that already has any `C` row is never rescanned. A week is frozen only if it starts at or after `week_start(S)` and is computed within 7 days of closing (Claude Code prunes old transcripts); otherwise its calls print `-` and nothing is stored. The current week's calls are always computed live and never stored.

## Behaviour

- `greenbelay rollup` prints one row per (week, rule), covering every rule in the rule table, newest week first. Text shows the current and the last closed week; `--weeks N` widens it; `--json` carries every week. A rule with no firings prints `0`. A rule with unknown calls prints `-`. A zero and an unknown are different things.
  ```
  week        rule                     firings   calls  rate/1000
  2026-11-23  path-operand-missing        3116   41210       75.6
  ```
- `rate` is firings per 1,000 calls of the rule's tool, for that week. Firings are live arm only. They sum denied, advised, watched and unconfirmed. Records at the `rules-checked` and `candidates-checked` outcomes are not counted, as in `tally`.
- `--json` schema (retro depends on it): `{"format":"greenbelay-rollup-v1","covered_since":"<date>","weeks":[{"week","partial":bool,"rules":[{"rule","firings","denied","advised","watched","unconfirmed","tool":"Bash"|"Read"|"any"|null (the `rule_tool` entry; `any` is the sum of all tools),"calls":n|null,"rate":x|null,"replay":"exact|none"}]}]}`.
  - `partial` is true for the current week, and for the first week when `S` falls after the Monday. Text prints the week cell as `2026-11-23 (partial)`.
  - `replay` is `backtest`'s label, so retro picks the source.
- The journal's still-unfolded rows are included, so the live week reads right before any rotation.
- Every state write finishes, and the lock is released, before the first byte goes to stdout. `Sub::Rollup` joins the SIGPIPE-ignored set (`src/main.rs:718`) and EPIPE is a quiet exit, so `rollup | head` leaves no lock or temp file.
- Exit 0 on success, warnings included (a failed `C` freeze write is a stderr warning); 1 for an unreadable state dir; 2 for usage errors. A missing transcript root is a warning on stderr naming the path tried and `--transcripts <dir>` (as `backtest` takes), and `calls` comes from the frozen `C` rows only. A progress line goes to stderr only when it is a TTY; `-q` silences it.
- `status`: the column label follows decision 2. `tally` returns the roll-up rows inside the window plus the journal rows inside the same window. Readers skip journal rows at or below the roll-up's `W`, so a crash before the journal delete, or a read racing a fold, never double-counts. A test covers both. Shadow rows stay out. When the journal holds a `rollup-failed` record newer than `W`, `status` prints one line naming the record and the file.

## Phases
- [x] Phase 1 — rename `status`'s `seen, last 30 days` to `seen since <date>` from `oldest_record()`; the first code commit, true today: `src/main.rs`, `CHANGELOG.md`
- [x] Phase 2 — the roll-up file (read, merge, atomic write, lock, watermark) and `rotate`'s best-effort fold, returning the fold failure beside the move result so `append` writes `F <ts> - - rollup-failed …`; the roll-up keeps 104 weeks of rows: `crates/greenbelay-guard/src/rollup.rs`, `crates/greenbelay-guard/src/journal.rs`, `crates/greenbelay-guard/src/lib.rs`
- [ ] Phase 3 — the per-tool transcript walker, the calls freeze, `rule_tool` and `greenbelay rollup [--json]`: `crates/greenbelay-guard/src/transcript.rs`, `crates/greenbelay-guard/src/rollup.rs`, `src/main.rs`, `docs/measuring.md`, `README.md`
- [ ] Phase 4 — `tally` reads roll-up plus journal; `status` switches to the 4-week label once coverage allows: `crates/greenbelay-guard/src/journal.rs`, `src/main.rs`, `docs/index.md`
- [ ] Phase 5 — release through the `tag-release` skill: `CHANGELOG.md`. Phases 1–2 ship first as their own release (fold only, no new command), because the roll-up counts only from its first fold and nothing before it comes back; Phases 3–4 follow in a second release.
- [ ] Phase 6 — retro's Target line reads `greenbelay rollup --json` when `replay` is `none`, instead of writing `uncounted`. Its own plan and PR in greenharness-skills, after the release, not part of this plan's PR. t34 stays open after this release; its Handoff names the skills plan. Close it in blue-factory's map only once that PR has merged and a retro Target line shows a roll-up count for a `replay: none` rule: `docs/programmes/blue-factory/t34-build-greenbelay-firing-rollup.md`, `docs/programmes/blue-factory/map.md`

## Verification
- Tests, in the crate (inline `mod tests`, with scratch dirs as the journal tests do):
  - A fold counts each record once across two rotations, and across a crash that re-folds the same generation.
  - Shadow rows stay out of `tally`.
  - A failed fold (a read-only roll-up path) still rotates, and `rollup-failed` is written.
  - A closed week's calls are not recomputed: the transcript is changed between two runs and the frozen count holds.
  - An unknown header version is skipped, with no panic.
  - A subagent transcript's calls are counted.
  - `rule_tool` covers every rule id.
  - `rollup`'s rule column uses `ui::id_column`, and a row fits in 80 columns.
  - The status label switches only when coverage allows.
- Pilot, on a copy of the live journal, never the live one. Set `CLAUDE_CONFIG_DIR` to a scratch directory, copy `journal.log*` into `<scratch>/greenbelay/`, symlink the real `projects/` transcripts, and drive the branch binary. It runs a forced rotation, then `rollup` and `rollup --json`, then `status`.
  - Expected: 14k firings in, the same total out of `rollup` plus the journal; sums by rule match the pre-fold `tally`.
  - Expected: the fold adds under 20 ms p95 to an `append` on a 256 KB generation; a crash injected between the roll-up rename and the journal delete leaves the total unchanged.
  - Observed, Phases 1–2 (scratch `CLAUDE_CONFIG_DIR`, branch binary, a copy of the live journal): one hook call over the 256 KB cap rotated; `rollup.log` took 1,270 firings in 77 `R` rows, equal to the 1,270 an awk count finds in the original generation 10; no `rollup-failed` record, no leftover temp file. `status` prints `seen since 2026-11-20`. Fold latency was not timed in this pilot, and no synthetic 104-week file was seeded: both are carried to the second release.
- `make check`, and the hook-latency pilot above.

## Overlap
Board 2026-11-23: `greenbelay-wt-preferred-model` (feat/agent-preferred-model) touches `src/hook.rs`, `src/rule_table.rs`, `rules/mod.rs` and `settings.rs`. It does not touch `src/main.rs` or `journal.rs`. `src/rule_table.rs` and `rules/mod.rs` are read here only. The expected merge conflict is none. Its new rule must appear in the `rule_tool` table, and the coverage test catches that if it lands first.

## Decision log
- 2026-11-23 — roll-up over longer journal (t29). The rule-to-tool table is explicit, because rules declare no tool and the rate needs one denominator per rule.
- 2026-11-23 — Phase 2 review: a journal over twice its cap rotates despite a busy roll-up lock and writes `rollup-failed lock-busy`, so a stuck lock cannot grow the journal without bound (architect, round 2). A roll-up of another format version is left untouched and reported as `rollup-failed rollup-format-unknown`. Records stamped a day ahead of the clock are left out and reported as `rollup-failed future-ts-<n>`. Same-second records at the watermark may undercount: kept, as the plan's decision 6 says. Carried to the second PR: `Arm` typed in `Key`, one shared bucket function with `tally`, a parent-directory fsync.

<!-- panel: repos=greenbelay,greenharness-skills,blue-factory adds=cli,contract reviewers=backend,lang:rust,tui,unix,architect,po body-sha=561128469e6a -->
