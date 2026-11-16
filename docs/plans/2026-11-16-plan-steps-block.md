---
plan-id: p-01M7HWWH16M0B7ATHHZQ7MKSDM
---
# B7: the "🧑 Your steps" block, with a steps count

## Review panel
👉 **Decide:** approve the growth, 602 → 1,063 words (×1.77): most of it fixes wording reviewers marked wrong or unverifiable, and the rest is the steps-parser detail; or ask for a smaller plan.
📍 amont-agent, greenharness-skills · 2 phases · 2 releases · next: Phase 1. Panel: backend, rust, tui, unix (all opus).
**Changed by review:** a steps state in the Phases parser, so a later Phase still counts; step format and warning fixed; plan_time.rs covered.
**Verdicts:** round 1: 4 approve-with-changes; round 2: 3 approve-with-changes, 1 approve; delta: backend resolved, 1 low. Carried: 2. Remove 0, merge 0.
**Carried into implementation:** the Picture caption should say a Phase after the block still counts (backend, low); the finish report restates the parser's rule, which a greenpiste subcommand would remove (unix).

## Picture
```
plan template ──► ### 🧑 Your steps   (only what the agent cannot run)
                  (🧑 decisions stay top-level items, counted as person:)

plan-panel ──► counts=…,words:N,steps:N,merge-candidates:…

worktree-task finish ──► last lines: the steps, or "Your steps: none."
```
The steps heading ends Phases for every Phases scanner, so its bullets are never phases, decisions or the next open phase.

## Context

The time-to-done programme (decisions t09, closed 2026-11-16) found that a plan's 🧑 items are mostly decisions. The steps a person must do (reinstall the Claude Code plugin, restart Claude Code, enter an OTP, set a secret, run a trust command at a real terminal, sudo) appear in plans only by accident. The fix is a `🧑 Your steps` block, shown only for actions the agent cannot run, and a `steps:` count so the panel line shows how many there are.

## Goal

- The plan template gains a `### 🧑 Your steps` block, inside `## Phases`, that lists only actions the agent cannot run, each with its exact command.
- `worktree-task`'s finish report ends with those steps.
- greenpiste's `counts=` token gains `steps:N`, placed after `words:` and before `merge-candidates:`.

## Non-goals

- No change to `person:` (decisions stay as they are).
- No relais change: relais reads `plan.counts` as an untyped value, so an added key is safe.
- No change to what counts as a decision.

## Code review

- amont-agent: backend, lang:rust
- greenharness-skills: backend

## Behaviour

**Template** (`greenharness-skills/parts/workflow/skills/worktree-task/plan-template.md`, Phases at lines 48-62): a `### 🧑 Your steps` heading inside `## Phases`, after the last phase. Each step is one plain top-level `- ` bullet (never `- [ ]`) with the exact command or action in backticks, then `— because <why the agent cannot run it>`. A `Phase N` item written after the block ends it and still counts as a phase.

**Parser** (`amont-agent/crates/greenpiste-process/src/file_types.rs`, `phase_spans` at ~138; `plan_phases.rs` `next_open` at ~503; `plan_time.rs` ~363): the heading `### 🧑 Your steps` is matched only outside fenced code, on its trimmed text. Each line gets one `Fence` helper call that yields both the `##` and the `###` case, because two calls on one line break fence tracking (`plan_text.rs:375-389`). At the heading the open phase is closed at that line and the walk enters an `in_steps` state (still inside Phases, `inside` stays true), so the last phase's lines, paths and wording stop there. In `in_steps` a top-level bullet is a step, not a phase or decision; a `Phase N` item leaves `in_steps` and is parsed as a phase. The block runs to the next `##` or `###` heading, or to a `Phase N` item. `next_open` stops at the same heading. A new `file_types::steps(body)` counts the top-level bullets of the block with the same fence-aware walk; a fenced example of the heading counts nothing. A slice keeps the steps block as shared text.

**Counts** (`amont-agent/crates/greenpiste-process/src/plan_review.rs`): `Counts` gains `steps`, from `file_types::steps`. `Display` always prints `…,words:N[/round1],steps:N,merge-candidates:…`, `steps:0` included. `to_json()` gains a `steps` key. The `counts_are_one_token…` test (~2480) keeps its `starts_with` pin, which still holds because `steps` comes after `words`.

**Warning:** `plan-panel` prints on stderr `plan.md:<line>: steps block outside ## Phases; not counted. Move it under ## Phases.` when a `Your steps` heading is found outside Phases.

**Worktree-task finish** (`greenharness-skills/parts/workflow/skills/worktree-task/SKILL.md`): after F10 step 3 (~line 497), the final report ends with the landed plan's block, under `Your steps (the agent cannot run these):`, one numbered line per step in the form ``N. `command` — because <why>``, so a copy of the backtick span takes only the command. With no steps the last line is `Your steps: none.` It reads the block by the parser's rule (fenced lines skipped, ends at the next heading or Phase item), not a sed range. The count printed equals `plan-panel`'s `steps:N`.

**Docs:** `amont-agent/docs/plan-review.md` (~101, 115-131) and `plan-review/SKILL.md` (~131) define `steps:` as "actions only the person can run; decisions stay in `person:`". They update the line-1 example, and state that readers take keys by name, that new keys go before `merge-candidates`, and that `merge-candidates` stays the last comma field. The HEADS.md example is stale and is left alone.

## Phases

- [ ] Phase 1 — greenpiste: the `steps` count, the parser stop, `next_open`, the warning, docs. `crates/greenpiste-process/src/file_types.rs`, `plan_phases.rs`, `plan_time.rs` (`phase_count` stops at the steps heading), `plan_review.rs`, `docs/plan-review.md`. Release greenpiste (minor).
- [ ] Phase 2 — greenharness-skills: the template block and the finish report; then pin the new greenpiste in grh, release skills and grh, and install them. `parts/workflow/skills/worktree-task/plan-template.md`, `parts/workflow/skills/worktree-task/SKILL.md`, and `plan-review/SKILL.md` (the counts line, in greenharness-skills).

## Decision log

- 2026-11-16 — Steps sit under `### 🧑 Your steps` inside Phases, not as top-level 🧑 items, because a top-level 🧑 already counts as a decision (`person`).
- 2026-11-16 — `steps:` goes after `words:`, so the existing `starts_with` test pin holds.
- 2026-11-16 — Round 1 (four seats, 16 findings) accepted in full. Phase 3 is folded into Phase 2: it installs what Phase 2 releases.

## Verification

- Phase 1, counts: input, a fixture plan with three phases, one 🧑 decision, and two steps (one written `- [ ]`) → `phases:3`, `person:1`, `steps:2`, and `next_open` returns Phase 3.
- Phase 1, fences: input, the same plan with a fenced example of `### 🧑 Your steps` inside a phase → `phases` and `person` unchanged, `steps:0`.
- Phase 1, stop: input, a `Phase 4` item placed after the block → it counts as a phase, and the block ends at it.
- Phase 1, range: input, the fixture with the steps block after Phase 3 → `phase_spans` for Phase 3 ends on the line before the heading; the step text containing "release" is in no phase's span (`phase_spans` and the `releases` count built on it); removing the close-at-heading line fails this test.
- Phase 1, time: input, a step whose text says "Phase 5" → `plan_time`'s phase count is 3, not 4.
- Phase 1, warning: input, the heading under `## Notes` → stderr carries the file name, the line number and the move, `steps:0`, and exit status 0.
- Phase 2, finish: input, a plan with steps `a`, `b` → the report's last lines are the two numbered steps; a plan without a block → `Your steps: none.`
- Phase 2, install: `greenpiste --version` and `grh --version` show the new versions; `plan-panel` on a plan with a steps block prints `steps:N` equal to the finish report's count.

## Outcome

## Full reviews (reference)
- backend: round 1 approve-with-changes (5); round 2 approve-with-changes (3); delta approve-with-changes (1, low).
- lang:rust: round 1 approve-with-changes (4); round 2 approve-with-changes (3, medium and low, carried or applied).
- tui: round 1 approve-with-changes (4); round 2 approve-with-changes (2, low, applied).
- unix: round 1 approve-with-changes (3); round 2 approve (none).
<!-- panel: repos=amont-agent,greenharness-skills adds=lang:rust reviewers=backend,lang:rust,tui,unix body-sha=2f02846765ca -->
