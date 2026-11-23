---
status: active
plan-id: p-01M84WTX4VTHDAWMEBT5WZ5748
branch: feat/size-boundary-handoff
repos: [greenbelay, greenharness-skills, decisions]
adrs: [ADR-0080, ADR-0083]
---
# Hand off mid-phase at about 400k context

## Review panel

📄 Full reviews: [2026-11-23-hand-off-mid-phase-400k.reviews.md](2026-11-23-hand-off-mid-phase-400k.reviews.md)
👉 **Decide:** none. Approve if one size handoff at about 400k (a deny, only if the replay holds) beats running on to 600k or more.
📍 decisions, greenharness-skills, greenbelay · 4 phases · 1 release · Phase 1 done (ADR-0083, decisions#352 merged; carried items for Phases 2–3 in decisions `docs/plans/2026-11-23-hand-off-mid-phase-400k.md`) · next: Phase 2, worktree-task Handoff mid-phase in greenharness-skills. Panel: architect, backend, lang:rust, po, tui, unix (opus), codex.
**Changed by review:** a new ADR replaces 0080 (no in-place edit); the size path runs before the early returns and is not a Candidate; the `deny` default survives the compiled `advise`.
**Verdicts:** 4 approve-with-changes, 2 approve; codex approve-with-changes; 12 carried; words 2128 → 2872 (×1.35, mostly fixes to wrong statements); remove 1 rejected, merge 1 accepted.

## Context

The 30-day token audit (2026-11-23, `~/.local/state/sess-audit/tokscan/`,
memory `prompt_cache_ttl_1h_via_gateway_2026_11_23`) found that cost comes
from context size, not from output. Opus calls above 500k context were 52% of
Opus spend. The 1h prompt-cache TTL (set 11-23) removed most of the re-cache
cost after idle gaps. The lever left is the size of the context each call
carries.

`clear-at-boundary` only hands off at a **command** boundary: a worktree
removed, a sweep, or a lone plan commit. A long implementation phase reaches
none of these, so a session can pass 400k, 600k or 800k mid-phase with
nothing to stop it. Of 98 clear-at-boundary checks, 57 were held, as a
pending background task or as no boundary at all.

The person chose (11-23, inbox 8efa358a) a **size boundary**: at about 400k
context, with nothing in flight, the session writes a mid-phase handoff
(the step underway and what it learned) and clears.

Evidence (measured 11-23 over 314 Opus main-chain sessions in 30 days,
$32.0k list-price spend; the script lands as `tokscan/size400.py`):

| threshold | sessions with a turn end past it | sessions idle past it | spend after the first idle turn end |
|---|---|---|---|
| 300k | 120 | 79 | $21.6k (67.6%) |
| **400k** | **101** | **63** | **$19.5k (61.0%)** |
| 500k | 67 | 44 | $17.0k (53.1%) |

- At the first turn end past 400k, 37 of 101 sessions had background work,
  27 asked a question, 8 had relais in flight, and 29 were idle.
- Of the 63 sessions idle past 400k, only 24 compacted later. The rest ran
  on at full size.
- 39 of the 63 had a command boundary in that same idle turn, yet nobody
  cleared. At `advise`, the note to the person is not acted on.
- In the 3 days the journal covers (11-20 to 11-23), clear-at-boundary
  watched 817 Stops:
  - 230 had no boundary;
  - 221 were pending work, meaning an open phase;
  - 226 were pending background.

  It denied 3.

The size boundary targets about 2 idle sessions a day. Those sessions carry
61% of spend from that point on.

Sibling plan: amont `docs/plans/2026-11-23-hand-off-before-long-queue.md`
(`queue-wait-handoff`). That plan also hands off before work finishes. It uses
the same Handoff section and the same `safe to /clear; next:` line, and its
`next:` names the exact step. This plan reuses that shape.

## Picture

```
Stop (turn ends)
  |
  +- silence? (plan mode, question, WAITING:, marker) -> silent
  +- context < 200k ----------------------------------> silent
  +- command boundary reached or pending
  |     existing tests; past 400k answered at the size
  |     stance (deny), below it at its own (advise)
  +- no command boundary, context >= 400k   <- NEW
        +- background task / relais in flight
        |     -> silent, journal `size held <why>`
        +- already fired at this 100k step -> silent
        +- fire at the size stance (default deny):
              Continue: write the mid-phase handoff,
              end with `safe to /clear; next: ...`
              plan-phases-open silent for the turn
              next Stop has the marker: both silent,
              the clear-relay clears and resumes
```

The picture shows where the size kind sits in the Stop judgement: after
the command-boundary path, which it does not change below 400k, and before
`plan-phases-open`, which it silences for the turn it fires in.

## Decisions (stated, recommendation taken)

1. **It is a kind inside `clear-at-boundary`, not a new rule.** The kind shares
   the transcript tail, the state file, `StopView` and the journal. Rows are
   journalled as `size …`, so they can be counted apart.
2. **Threshold: 400k tokens.** Git config key
   `greenbelay.clear-at-boundary.size-tokens`, which may not go below `SIZE`
   (200k); a value that does not parse or is below `SIZE` is reported on
   stderr through `gitconfig`'s complain path and falls back to 400k. Break-even under the 1h TTL: a fresh session writes its prefix R
   once at the 1h write price, 2× base or 20 cache reads. The session that
   keeps going pays `S − R` more per turn. A clear repays itself after
   `k = 20·R/(S − R)` turns. A mid-phase resume re-reads more than a bare
   prefix (plan, diff, files), so take R ≈ 150k. At S = 400k, k = 12 turns,
   and the long phases this targets run for far more. A mid-phase handoff
   also loses working knowledge that a command boundary does not, which is
   why the threshold is not 200k. The `SIZE` doc comment gets the 1h
   arithmetic too.
3. **What counts as in flight at a size boundary.** These still block:
   - a background task (`background_busy`) or a relais run waiting
     (`relais_flight`);
   - an unanswered question, a `WAITING:` line, or plan mode.

   **Dropped for this kind:** the clean-repository and nothing-unpushed
   tests, and ADR-0080's "a worktree left to tear down" and "an open PR of
   its own": the task worktree is the state, and the handoff names the PR.
   A PR whose checks are still being polled is a poll, so it blocks. Mid-phase, uncommitted edits are the normal state. The task
   worktree survives a `/clear`, and the handoff names those edits. A WIP
   commit would put half-done code through pre-commit gates. A driver with
   its agents running is held by `background_busy`, which keeps ADR-0080's
   "never hand off mid-run".
4. **Ordering with `plan-phases-open`.** At `Working` (an open phase with
   commits after the plan), a size fire comes before `plan-phases-open`. That
   rule then stays silent for the rest of the turn. This widens the existing
   `spoke_this_turn && PlanOnly` exception in `src/hook.rs:130` to a size fire
   at any phase fact. Without it, `plan-phases-open` would send the agent back
   to work after the handoff.
5. **`stop_hook_active`.** Today any Stop in a turn that a hook continued is
   silent (`StopView::says_silence`). Mid-phase, most Stops after the first
   are of that kind. The size kind is judged on them too, at most once per
   turn (the state file's turn record), so a phase that `plan-phases-open`
   keeps alive still meets the size check.
6. **Dedup.** The kind fires at the first idle Stop at 400k or more, then
   again at each further 100k step (500k, 600k), up to 3 per session. That
   counter is separate from the 3-advice cap on command boundaries. After a
   real `/clear`, a new session id starts at zero.
7. **Stance.** The kind has its own key,
   `greenbelay.clear-at-boundary.size-stance`, default `deny`:
   - `deny`: a Continue to the agent.
   - `advise`: the same note to the person that a command boundary gets.
   - `observe`: journal only.

   The effective size stance is `size-stance` (default `deny`), forced to
   `observe` by `$GREENBELAY_OFF`, `greenbelay.enabled=false`, the shadow
   arm, or an explicit `greenbelay.clear-at-boundary.stance observe`, and
   capped by `max_stance`. The rule's compiled default `advise`
   (`rules/clear_at_boundary.rs:29`) does not lower it: otherwise every size
   fire would be an advise. Test: with no config, a 450k stub gets a
   Continue; with `GREENBELAY_OFF=1`, silence and an `observe` row.

   It ships at `deny` if the replay meets its bar (Phase 3), else at
   `observe`. At `advise`, a note to the person would arrive while
   `plan-phases-open` keeps the agent working, and that is incoherent. The
   clear-relay's 10-second countdown, which `/stay` can stop, is the person's
   brake. Rollback: `git config --global greenbelay.clear-at-boundary.size-stance observe`.
7b. **A command boundary past 400k is judged at the size stance.** 39 of the
   63 idle sessions past 400k reached a command boundary at that point. The
   command kind's `advise` note did not clear them. Past `size-tokens`, a
   command boundary that passes its existing tests is answered at the size
   kind's stance, `deny`. Between 200k and 400k the command kind stays at its
   own stance, `advise`.
8. **Where the handoff goes and what it holds.** It goes in the repository,
   never in memory (ADR-0080):
   - **In a programme:** the ticket's own `## Handoff` (the ticket file on its `prog/<slug>/<id>` branch), never the map's shared section: tickets run in parallel, and one shared section would be overwritten and conflict at merge.
   - **Otherwise:** a `## Handoff` section in the plan, replaced each time.

   It holds the date, `mid-phase at <N>k`, the phase and the step underway,
   what that phase has committed, the uncommitted files and their state,
   what was learned (facts found, dead ends), and the next action. It is
   committed alone as `docs(plan): handoff <phase>`, in the task worktree (the command-boundary detector at clear_boundary.rs:1319 excludes this subject the way it excludes closing plan commits, so the handoff commit never becomes a pending command boundary),
   unpushed.
9. **The next prompt names the same work.** `relais programme next --claim`
   is not used, because it picks the next unheld ticket.
   - **Outside a programme:** ``safe to /clear; next: `resume <phase>: <gist>; worktree <abs worktree>, plan <plan file>; read its ## Handoff` `` (phase and gist first: the countdown shows only the first 30 characters).
   - **In a programme:** ``safe to /clear; next: `resume <repo> <programme> <ticket>: mid-phase, <gist>` `` (the full marker line: the clear-relay arms only on `^safe to /clear; next: `(.+)`$`).
     The ticket stays held by its `prog/<slug>/<id>` branch across the
     clear. The resumed session refreshes the claim.
10. **Repositories and order.** Three repositories, one PR each:
    - decisions first: the new record governs.
    - greenharness-skills next: the procedure the Continue points at.
    - greenbelay last: the rule, then a release.

    The skill text does not name greenbelay (every part is optional): it
    reads "at a size boundary, when told".

## Phases

### Phase 1 — a new ADR replaces ADR-0080 (decisions repo, one PR)

- A new ADR, the next free number (0083 today), `docs/adr/0083-a-session-hands-off-at-its-size.md`, whose `work.programme-map` has `replaces: [ADR-0080]` and restates the choice whole (the corpus changes a decision by a new record: 0080:6,13, 0057:13). It adds a sixth boundary, the **size boundary**. A
  session whose main context is past about 400k, with nothing in flight
  except uncommitted work in its own task worktree, hands off mid-phase.
  State the handoff content of decision 8 and the prompt forms of decision 9.
  Restate the choice text, the Decision section and Consequences, and add the
  reason: 52% of Opus spend was above 500k, and the person chose this on
  11-23.
- `HEADS.md` and `aval.pack`: regenerate them the way the repository does,
  and check with `aval resolve` that the record still resolves.
- The clear-at-boundary amendment is the greenbelay `docs/rules.md` row
  (Phase 3). The decisions repository has no separate record for it; check
  `aval relevant` for one before writing.

### Phase 2 — worktree-task Handoff, mid-phase (greenharness-skills, one PR)

- `parts/workflow/skills/worktree-task/SKILL.md` `## Handoff`:
  - **When:** add boundary 5, "a size boundary: the context is past about
    400k mid-phase, when a hook or the person says so". Its in-flight list
    drops the uncommitted-work item (decision 3).
  - **Steps:** add a mid-phase variant, covering the handoff content
    (decision 8), the commit alone, and the prompt forms (decision 9),
    with no `next --claim`.
- Run the skill-trigger tests and lint the repository runs.

### Phase 3 — the size kind in clear-at-boundary (greenbelay, one PR)

- `crates/greenbelay-process/src/clear_boundary.rs`:
  - `SIZE_BOUNDARY` (400k), with its doc comment holding the 1h arithmetic.
    Fix `SIZE`'s comment to the 1h write price.
  - A size path in `decide`, before the `says_silence` return (:2314) and the `Working` return (:2321), and in `on_stop` before `payload_says_silence` (:1917). It is not a third `Candidate` in `Ledger::judge`, which `Working` never reaches. It is judged when this turn reaches no command boundary; a pending boundary held on a dirty tree does not block it. It reuses
    `context_size`, `relais_flight` and `background_busy`, and never calls
    `check_repositories`.
  - The state file grows `size_steps_fired`: `parse_state` reads it with `.as_u64().unwrap_or(0)`, `render_state` writes it, the text format reads 0, `StateFixture` gets `#[serde(default)]`, and the command path's `State` literal keeps it with `..state`. Test: an old file parses and the count survives a command advise.
  - The `stop_hook_active` exception (decision 5): the size check runs ahead of `payload_says_silence` in `on_stop` (:1917), `says_silence` in `decide` (:2314) and the `Working` return (:2321); plan mode, a question, `WAITING:` and the marker still silence it.
  - Message text through `decision::phrase`: it names the context size, the
    worktree-task Handoff mid-phase steps and both prompt forms, and says
    "keep uncommitted edits; list them in the handoff". It tells the agent to open its answer with one line for the person, `size boundary at <N>k: handing off mid-phase (off: git config --global greenbelay.clear-at-boundary.size-stance observe)`, and to make the marker line the last non-empty line.
  - Journal rows: `size fired <N>k`, `size held background|relais`,
    `size watched cap`, journalled with the size stance in `verdict.stance`.
  - Persist size-decision evidence independently of journal rotation for at least 14 days: session id, turn/Stop identifier, timestamp, context size, boundary kind, effective stance and outcome. Include command-boundary responses promoted by decision 7b. Deduplicate repeated records and correlate actual handoff requests with clear-relay submissions by session and time. Before release, verify with forced journal rotation that the seven-day follow-up can still recover every request and its matching submission; report missing evidence separately from an unheeded request.
- `crates/greenbelay-guard/src/rules/clear_at_boundary.rs`: the two config
  keys, read like the existing stance key.
- `src/hook.rs` `on_stop`: widen the silence of `plan-phases-open` after a
  size fire (decision 4), with the exact predicate stated; test that `plan-phases-open` still speaks at `Working` when there was no fire.
- The corpus `clear-at-boundary.cases`, plus Stop fixtures for:
  - a size fire at `Working`;
  - held on background;
  - silent below 400k;
  - a second fire only at 500k;
  - a command boundary at 450k answered at the size stance (Continue), and one at 300k still at `advise` (decision 7b);
  - the marker turn, where both rules are silent.
- `tests/clear_at_boundary.rs`: integration tests for the same cases through
  the hook binary, including `plan-phases-open` staying silent after the
  fire.
- `docs/rules.md` row and `clear-at-boundary` paragraph, `CHANGELOG.md`, and
  the AGENTS.md block (`amont agents-md`).
- Run the observe replay (Verification) before the release. The default
  `size-stance` ships as `deny` only if the replay meets its bar; otherwise
  it ships as `observe` and the follow-up ticket re-judges it.
- Release a minor version through `tag-release`, then `greenbelay install`
  on this machine.

### Phase 4 — follow-ups

- In `greenbelay/docs/programmes/follow-ups/`, committed in the Phase 3 PR, a `task` ticket due 7 days
  after the release:
  - count `size fired` rows;
  - count how many were heeded (the same turn ends with `safe to /clear`);
  - count clear-relay `submit` lines after a size fire;
  - rerun `tokscan/size400.py` for the share of Opus spend above 400k
    against the baseline above.

  Target: the spend after the first idle turn end past 400k falls from
  61.0% to at most 50%. Rollback to `observe` if fewer than 50% of
  `size fired` rows are followed by a clear-relay `submit`, or if Opus
  list-price spend per active session-day rises against the 30-day
  baseline ($32.0k / 314 sessions), or if more than 1 of 5 resumed
  sessions, read by hand, redid work the handoff named as done.
- File the inbox lines of session 8efa358a and the orphan into the same
  follow-ups map:
  - the 1h TTL check, `due=2026-11-30`;
  - the 4bca21ec flaky `tests/clear_at_boundary.rs` test. This plan edits that
    file, so expect the flake on a jammed host.

## Non-goals

- No automatic `/clear` from greenbelay. The clear-relay stays the only
  thing that clears, and `/stay` stops it.
- No change to what blocks a command boundary below 400k. The command kind
  keeps its tests and its 3-advice cap.
- No context-size lever for subagents or relais workers. They are
  short-lived and about 3% of spend.
- No hand-off while background work runs. That is ADR-0080's "never mid-run",
  and the size kind holds then.

## Verification

- greenbelay: `cargo test` (the whole workspace) and `greenbelay corpus check`.
- Replay: add a transcript-driven chronological runner for the size kind over the 30-day transcripts; do not restrict inputs to the existing exporter's journal-selected command boundaries. Include no-boundary Stops and hook continuations, carry simulated size state between Stops, and reconstruct background, relais, question, WAITING, marker and permission-mode facts at each Stop. Preserve the production transcript-tail limits. Report coverage and unavailable facts explicitly, with separate results for mid-phase hook continuations. Where historical facts cannot be reconstructed, collect representative live observe evidence before graduation; incomplete coverage cannot qualify the default for deny. Report fires per week and the share of first fires
  that were idle by the agent's classification. Bar: at least 1 fire per
  day across the fleet, and at most 10% of fires held wrongly: a background task, relais run or question in flight at the fired Stop per the transcript (input: the 30-day transcripts; expected: about 2 fires per day, none in the 37 sessions with background work; actual: reported). An editing turn after a fire is normal mid-phase and is not a miss.
- Live: in a scratch worktree with an open plan phase, a stub transcript at
  450k. The hook emits the Continue once, the next Stop with the marker is
  silent for both rules, and a 350k stub is silent.
- clear-relay: `plugin/clear-relay/hooks/register.test.ts` cases for both
  decision-9 lines: each arms, the programme form logs `named` with its
  ticket, and the other form logs `named: null`.
- skills: the repository's own checks. decisions: `aval resolve
  work.programme-map` names the new record.

## Code review

- greenbelay: backend, lang:rust, unix, tui, architect
- greenharness-skills: none — markdown only
- decisions: none — markdown only

<!-- panel: repos=greenbelay,greenharness-skills,decisions adds=contract reviewers=architect,backend,lang:rust,po,tui,unix,codex body-sha=4d21888b103f -->
