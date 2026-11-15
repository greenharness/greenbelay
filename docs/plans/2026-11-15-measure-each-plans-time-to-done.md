---
status: active
branch: feat/plan-time-to-done
repos: [amont-agent, relais, dotfiles, decisions]
adrs: [ADR-0070, ADR-0064]
---
# Measure each plan's time to done

## Review panel
👉 **Decide:** none. Approve if a session-and-window join (relais keeps no branch) and keying on the machine comment's `body-sha` (75 of 90 resolve) are good enough to start measuring.
📍 amont-agent, relais, dotfiles, decisions · nothing built · next: Phases 1 and 2 in parallel, by background agents. Panel: backend, lang:rust, tui, unix, architect, po (all opus-5-5). codex: not installed.
**Changed by review:** start key moved from landed sha (17/157) to machine comment; `report runs` subcommand with `schema: 1`; unreviewed plans out of medians.
📄 Full reviews: [2026-11-15-measure-each-plans-time-to-done.reviews.md](2026-11-15-measure-each-plans-time-to-done.reviews.md)
**Verdicts:** approve-with-changes ×5, then all six approve on the final body; 3 po findings rejected (Decision log); 10 carried into implementation (below). Words 1355 → 2106 (×1.55).

Carried into implementation:
- One entry counts for at most one plan. The pre-`add` window applies only to the session that runs the add, and the latest start wins (backend, unix, rust).
- A test that `plan-review stats --nosuchflag` exits 2.
- `src/main.rs` writes stats with `write_all` and maps `BrokenPipe` to exit 0, with a test (rust).
- Fixtures cover: a path reused after `worktree remove`, `--detach` then `switch -c`, and a pre-`add` entry whose `cwd` maps to another plan (rust).
- Error lines follow `greenpiste: <what>: <path>: <reason>. Run: <command>`.
- Progress is off with `--json`.
- A failure line in retro-weekly sits in a code span, with backticks stripped (tui).
- The Phase 3 verification sets `RETRO_DIR`.
- A `schema` mismatch in retro-weekly prints the same one line as an older tool (tui, unix).
- relais checks the ledger with an already-migrated ledger or a `usage_events` count, not mtime alone (rust).

## Picture
```
 transcripts (*.jsonl) ───────┐
 greenpiste ledger + bodies ──┼─► greenpiste plan-review stats --time
 git: docs/plans/*.md ────────┘   per plan: start, done, blocked h,
                                  person h, stops, words, sessions
                                              │
 relais ledger ─► relais report runs --json  │
                  per run: root_session, hours│
                              │               │
                              ▼               ▼
                  retro-weekly.sh: jq join on session id + window
                              │
                              ▼
                  ~/.claude/retro/<date>.md, "Time to done"
```
Three sources feed greenpiste's per-plan rows; relais publishes run hours on its own; retro-weekly joins the two by session and prints the table.

## Goal
The time-to-done programme needs a measure before its review rules can be judged: per plan, how long from the first panel round to `status: done`, and how much of that was agent-side waiting (gates, relais runs, review agents) rather than waiting on the person. This builds the measure t01 specified (decisions `docs/programmes/time-to-done/t01-what-time-to-done-measures.md`, ADR-0070 `work.plan-time-to-done`). Its first run recomputes t00's provisional baselines and answers the person's original question about `greenpiste-data-names-rename` and `plan-duro-settings-rollout`.

## Non-goals
- A time estimate on the panel line (needs four weeks of this data; fog in the map).
- The `counts=` panel line (B5), background gates (B2), and the subtraction rule (B3, built in parallel).
- Reading another tool's files: greenpiste reads transcripts, its own ledger and git; relais publishes its run hours as a query (ADR-0064, `harness.cross-tool-reads`).
- Deleting the `~/.local/state/sess-audit/` prototypes: they stop being used once this ships, and they are outside every repository.

## Code review
- amont-agent: backend, lang:rust, unix, tui
- relais: backend, lang:rust, unix, tui
- dotfiles: unix (the tool proposes none: a chezmoi `executable_*.sh` name is not detected; it is a POSIX sh script)
- decisions: none — markdown only

## Behaviour

### relais: `relais report runs --json [--since <date>]`
One JSON object `{"schema": 1, "since": …, "runs": [...]}` (`runs: []` with exit 0 when empty; retro-weekly checks `schema`; `--since` defaults to the start of the month in UTC, as `report` does), one object per run started in the window: `id`, `repo` (`runs.repo_path`), `root_session`, `status`, `started_at` (the earliest `attempts.started_at`, else `runs.created_at`), `ended_at` (the latest `attempts.ended_at`, else `runs.updated_at` for a terminal run, else null), and `hours`. It is a subcommand of `report` with its own args, not a mode flag: it never runs `report_command`'s usage import, routing, learning or other sections, so it is read-only, and later options (by repo, by status) need no conflict entries on `report`. It has no text form: `--json` is required, so `report runs` without it exits 2. Branch is not stored by relais, so the join key is the root session plus the time window, not the branch.

### greenpiste: `greenpiste plan-review stats --time [--since <date>] [--json]`
- **Plans.** Every `docs/plans/<date>-<slug>.md` under the plan root (`plan_review::locate`), except `.reviews.md` and pointer files, whose front matter carries `branch:`. A plan is keyed by (repository, branch). A plan without `branch:` goes to an exclusion count.
- **Start.** The key is the `body-sha=` in the landed plan's machine comment (the plan file or its `.reviews.md`), as t01 specifies. When there is none, the key is the landed body's sha by `plan_text::body_sha`, taken from the file at its first commit. Measured 2026-11-15 on 157 landed plans with `branch:` since 11-01: 90 carry the comment, 75 of those resolve in the ledger, and the landed-body sha alone resolves only 17. Find the reviewer rows whose `body_sha` or `first_round_sha` equals it, or a plan-apply merge row whose `body_sha_after` equals it, followed through its `body_sha_before` to a reviewer row (merge rows carry no `first_round_sha`), and take the reviewer row's `first_round_sha`. The start is the earliest `recorded_at` of a round-1 row with that `first_round_sha`. A plan with no matching ledger row starts at its first commit and is flagged `unreviewed`.
- **Completion.** The commit date of the first commit whose front matter has `status: done` (`git log --reverse -G'^status: done'`). A plan with no such commit is listed as unfinished, with its age, and is never counted in medians.
- **Attribution.** Each transcript entry's `cwd` is mapped to (repo, branch). The `gitBranch` field is not used, because it is frozen at the session's start: 1412 of 1412 worktree entries sampled on 2026-11-15 read `HEAD`.
  - The path→branch map comes from two sources: the `git worktree add … -b <branch> <path>` commands in the transcripts (paths resolved against that entry's `cwd`), and `git worktree list --porcelain` for worktrees that still exist. The map is keyed by (path, time): a path maps to the branch of the latest `worktree add` before the entry, and is closed by a later `worktree remove`. A `--detach` worktree maps to the branch named by its first `git switch -c`/`checkout -b` in that cwd, else to unattributed.
  - A session's entries from the plan's start up to the `git worktree add` that creates the plan's branch belong to that plan, whatever their `cwd`.
  - Any other entry whose `cwd` maps to no plan goes to the `unattributed` row.
- **Blocked wait.** The union of the plan's intervals, with overlaps counted once:
  - a foreground `Bash` gate, test or build call (the `waits.py` regex, with `greenpiste`/`greengate` names added), from `tool_use` to `tool_result`;
  - a foreground `Agent` call;
  - an `mcp__relais__*` call;
  - a gap over 600 s ended by a `queue-operation` or task notification.

  `resource_h` is the plain sum of the same intervals.
- **Other columns.**
  - `person_h`: gaps over 600 s ended by a person's message.
  - `stops`: `ExitPlanMode` calls.
  - `questions`: `AskUserQuestion` calls.
  - `prs`: `gh pr create`, or a POST to `…/pulls`.
  - `releases`: a `git tag v…` or `land --tag`.
  - `words` and `phases`: round-1 body (`bodies/<first_round_sha>.md`) → landed body.
  - `sessions`: `[{id, from, to}]` per plan, for the join.
- **Text output.** Fits 80 columns: plan (`repo/branch`, cut with `…` at 28 characters), elapsed h, blocked h, person h, stops, words ×. The other fields, `sessions` included, are `--json` only. One row per finished plan, then the unfinished plans with their age, the `unattributed` row, the exclusions with counts (no `branch:`, no saved round-1 body, entries with no `cwd`), and a summary line: plans counted, plans flagged `unreviewed`, median `blocked_h`, median word growth, the share grown past ×1.25, and the greenpiste version. Elapsed and blocked medians cover reviewed plans only (an `unreviewed` plan starts at landing, so its elapsed time leaves out the panel); `unreviewed` plans are rows of their own, shown separately and never in a median.
- **`--json`.** The same data as one object: `schema` (1), `plans`, `unfinished`, `unattributed`, `exclusions`, `summary`. Only the JSON goes to stdout.
- **Exit codes** (in `USAGE`): 0 on success, even with zero plans; 1 when the ledger or the transcripts cannot be read; 2 on a usage error (a bad `--since`, or `--time` with `--model`). A repository where git fails gets one line on stderr and an `exclusions` count. A closed stdout pipe exits 0 (as in board.rs:225).

### retro-weekly
The weekly report gains a `## Time to done` section:
- it runs both queries for the last 7 days; `--since` keeps a plan done in the window, or unfinished, whenever it started;
- `jq` joins relais run hours onto each plan, matching a run's `root_session` against the plan's session ids, with `started_at` inside that session's window;
- it prints a table: plan, elapsed, blocked h, relais h, person h, person stops, words ×, with a note that relais h overlaps blocked h (foreground relais calls) and is not added to it.

If either tool is absent (`command -v` fails) or older (exit 2, an unknown flag), the section says so in one line and the report goes on. Any other non-zero exit prints one failure line with the last stderr line. The window starts at `/bin/date -v-7d +%F`. The section is built in a temp file that the existing EXIT trap removes.

## Phases
- [ ] Phase 1 — relais `report runs`: `crates/relais/src/cli/mod.rs`, `crates/relais/src/report/mod.rs`, `crates/relais/tests/report_runs.rs`, `CHANGELOG.md`. Release a minor version (tag-release). Built by a background agent.
- [ ] Phase 2 — greenpiste `stats --time`, in parallel with Phase 1:
  - progress (`transcripts N/M`) on stderr only when stderr is a TTY; nothing when it is not;
  - a new module `crates/greenpiste-process/src/plan_time.rs`, with the flag and usage in `crates/greenpiste-process/src/review_outcomes.rs` (`run_stats`, `USAGE`) and `src/main.rs`;
  - fixture tests in `tests/plan_time.rs`: transcripts, a temporary git repo with a landed plan and a done commit, and a ledger, using the `tests/outcomes_stats.rs` pattern; the transcripts include `git worktree add -q -b <branch> ../x-wt-y` (a relative path resolved against `cwd`), a `--detach` add, and a removed worktree whose entries go to `unattributed`;
  - timestamps (`--since`, transcript ISO times) parsed with `greenpiste_guard::civil::days_from_civil`, with no new date crate; `--model` with `--time`, and `--since` without `--time`, are usage errors;
  - `docs/plan-review.md` and `CHANGELOG.md`;
  - release a minor version. Built by a background agent; it rebases over the in-flight `feat/greenpiste-data-names` branch if that lands first.
- [ ] Phase 3 — after both are released and installed on this machine, add the retro-weekly section in the chezmoi source `dot_claude/tools/retro-weekly/executable_retro-weekly.sh`. `chezmoi apply` that one file, then run the script once by hand with `RETRO_DIR=$(mktemp -d)`, so today's report is not overwritten.
- [ ] Phase 4 — the first 30-day run, written into decisions:
  - `docs/programmes/time-to-done/t00-time-audit.md`: the recomputed baselines, with coverage, exclusions and the greenpiste version;
  - `docs/programmes/time-to-done/t01-what-time-to-done-measures.md`: the target, re-set to 60 % of the new median of reviewed plans, with the old 0.5 h target and the reason recorded beside it;
  - the two plans the person asked about. If either is not a landed plan, say so and give its closest match;
  - `docs/programmes/time-to-done/map.md`: B1 built, in the Build queue and Notes.

## Decision log
- 2026-11-15 — Attribution uses the transcript's `cwd` mapped through `git worktree add` commands, not `gitBranch`, because `gitBranch` stays at the session's start branch. 1412 of 1412 worktree entries sampled read `HEAD`.
- 2026-11-15 — relais keeps no branch, so the join is by `root_session` plus the session's window on the plan, not by branch. t01's wording ("per repository and branch") is met through the session.
- 2026-11-15 — Start keys on the machine comment's `body-sha=`, then the landed-body sha: the landed sha alone resolved 17 of 157 plans, the comment 75 of the 90 that carry one. Plans with no ledger match are `unreviewed` and stay out of the medians.
- 2026-11-15 — Both JSON outputs carry `schema: 1`, and the relais query is a `report runs` subcommand (architect), because a tool read by another needs a version token (`harness.cross-tool-reads`), and a mode flag would have to switch off every section of `report`.
- 2026-11-15 — Kept against the product owner: Phases 1 and 2 build in parallel (the person's ask; retro-weekly degrades without the relais column); the `prs` and `releases` columns stay because t01 names them.
- 2026-11-15 — Plans without `branch:` front matter are excluded and counted, not guessed. In the last two weeks, 119 of 227 landed plans carry `branch:`. The template requires it, so coverage grows.

## Verification
- Phase 1: on a fixture ledger with 2 runs (one terminal, one running), `relais report runs --json` → 2 objects, the running one with `ended_at: null` and `hours > 0` (or `now` injected, so the test does not depend on the clock). On the live ledger, `relais report runs --json --since 2026-11-08 | jq '.schema, (.runs|length)'` → `1` and a count above 0; the ledger's mtime is unchanged.
- Phase 2: the fixture with one plan, two sessions and one overlap → `blocked_h` equals the hand-computed union, and `resource_h` is greater than `blocked_h`. A plan without `status: done` → listed under unfinished. On the live data, `greenpiste plan-review stats --time --since 2026-10-16` → at least 10 finished plans, with `unattributed` and `exclusions` printed; unattributed hours ≤ 25 % of all hours, and plans flagged `unreviewed` at most 55 % of finished plans, the 2026-11-15 measure being 82 of 157, which shrinks as plans land under the template (the summary line prints both, plus the number of finished plans with a round-1 body; under 4, the t10 check is reported as insufficient); the run completes in under 60 s (timed).
- Phase 3: `~/.claude/tools/retro-weekly/retro-weekly.sh` → the report file has a `## Time to done` table with at least one row whose relais h is above 0, and a line giving relais hours whose `root_session` matched no plan session, out of the total.
- Phase 4: t00 gives three recomputed baselines next to the provisional ones (0.13, 0.82 h, 1). `relais programme check docs/programmes/time-to-done` exits 0.

## Outcome

<!-- panel: repos=amont-agent,relais,dotfiles,decisions adds=cli,contract reviewers=backend,lang:rust,tui,unix,architect,po body-sha=19053529d9c2 -->
