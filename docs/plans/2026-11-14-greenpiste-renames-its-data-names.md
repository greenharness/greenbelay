---
status: active
branch: feat/greenpiste-data-names
repos: [amont-agent, relais, amont, grh, greenharness-skills, dotfiles, decisions]
adrs: [ADR-0068, ADR-0064]
---
# greenpiste renames its data names

## Review panel

👉 **Decide:** none — approve if greenpiste 5.0 hard-renames its data names (ADR-0068) and migrates this machine once, with hooks never blocked and old records still read until the census.
📍 amont-agent, relais, amont, grh, greenharness-skills, dotfiles, decisions · nothing built · next: Phase 1 (relais and greengate read both). Panel: backend, lang:rust, lang:typescript (opus-5-5), codex.
**Changed by review:** hooks never block. A done-marker, a reversed-marker and a failed-marker are added, and old keys are read until the migration is done. Keys are renamed in their real config files, includes kept. `--reverse` rewrites 5.0 rows back, and recovery commands skip the forward migration.
📄 Full reviews: [2026-11-14-greenpiste-renames-its-data-names.reviews.md](2026-11-14-greenpiste-renames-its-data-names.reviews.md)
**Verdicts:** round 1 approve-with-changes 3; round 2 approve-with-changes 3 (all carried); Codex approve-with-changes (2 high applied); final binds approve 3 after one backend blocker; 0 rework; 9 carried into implementation.

## Picture

```
 kind            today (4.0.1)                  after this plan (5.0.0)
 ----            -------------                  -----------------------
 state dir       ~/.claude/amont-agent/         ~/.claude/greenpiste/
                 (730 MB, journal, ledgers)     old path: a symlink
 state root      ~/.local/state/amont-agent/    ~/.local/state/greenpiste/
                 (plugin marketplace, relay)    old path left as a symlink
 git config      amont.agent.<key>              greenpiste.<key> (moved)
 env             AMONT_AGENT_OFF, _PLAN_ROOT…   GREENPISTE_OFF, _PLAN_ROOT…
 headers         amont-agent-v1, -review-*-v1   greenpiste-v1, -review-*-v1
                                                (old rows still read)
 record fields   "tool":"amont-agent",          "tool":"greenpiste",
                 producer amont-agent <v>       producer greenpiste <v>
 marketplace     clear-relay@amont-agent        clear-relay@greenpiste

 1 relais, greengate, greenharness-quiet read both
 2 greenpiste 5.0 writes new; its first run (under a lock) moves
   the roots, keys and plugin once; `migrate --reverse` undoes it
 3 skills, grh, dotfiles say the new names
 4 record; the census removes every old read
```

greenpiste writes only the new names; this machine's state and config move
once, under a lock; old records and committed files are still read until
the programme's census.

## Context

Programme gate-guardrails, step 3b-2. 3b-1 (greenpiste
`docs/plans/2026-11-14-amont-agent-ships-as-greenpiste.md`, done) renamed
the binary, crates and packaging; every name greenpiste reads or writes
still says `amont-agent`. The person's end state (2026-11-12): "no amont,
only green*". ADR-0068 sets the path for a tool with no outside users:
hard-rename data names, migrate the person's own machine once (config keys,
ledgers and state moved under a lock, refused with what to fix, reversible
by name), keep one fallback read for committed files until a fleet sweep,
and move state directories once, leaving a symlink at the old path.
greengate did the same as plan 3a-2
(`amont/docs/plans/2026-11-11-greengate-renames-data-names.md`).

An inventory (2026-11-14) found:
- two state roots: `~/.claude/amont-agent/` (`journal::dir()`,
  `config_dir().join(DATA_DIR)`, `journal.rs:56`; 730 MB, 689 MB of it
  attestations) and `<state>/amont-agent/` (`settings::state_root()`,
  plugin marketplace and clear-relay sessions; the TypeScript plugin
  hard-codes `~/.local/state/amont-agent` in
  `plugin/clear-relay/hooks/register.ts:102` and `plugin.json:16`);
- git config keys built under `PREFIX = "amont.agent."`
  (`gitconfig.rs:118`, `stance.rs`, `shadow.rs`): 30 global keys on this
  machine, among them one git reports lowercased (`denyagents`);
- env vars read at call sites (`AMONT_AGENT_OFF` `stance.rs:45`,
  `_PLAN_ROOT`, `_GH`, `_GH_DEADLINE_MS`, `_AVAL`, `_ADR_LINT`, `_PROBE`,
  `_TEST_*`, `_BIN`); none is set on this machine;
- headers and schema strings kept in every old row: `amont-agent-v1`
  (journal; a different header reads as an empty journal),
  `amont-agent-cases-v1` (57 committed corpus files),
  `amont-agent-review-outcome-v1`, `-review-merge-v1`,
  `-review-disposition-v1`, `-impl-review-outcome-v1`, and the provenance
  `producer amont-agent <v>` line;
- readers in other tools: relais `review_outcome.rs` (ledger path parts,
  two schemas, `AMONT_AGENT_PLAN_ROOT`) and `quiet.rs` (the
  `.amont-agent-tmp-*` skip); greengate `attest.rs` (`provenance_dir` =
  `<config>/amont-agent/provenance`) and `greenharness-quiet/src/lib.rs:212`
  (the same temp skip and the `"tool":"amont-agent"` field);
- written instructions naming paths and keys: 4 skills/agents in
  greenharness-skills, `dot_claude/CLAUDE.md.tmpl` and
  `dot_claude/tools/retro-weekly/executable_retro-weekly.sh` in dotfiles,
  grh's `scripts/e2e.sh` leftover allowance;
- nothing migrates state today, and `greenpiste doctor` reports no data
  leftovers.

## Goal

greenpiste 5.0.0 reads and writes only `greenpiste` data names; this
machine's state, git config and plugin registration are moved once; relais
and greengate read the new names (and the old until the census); no skill,
dotfile or grh allowance tells an agent to use the old ones.

## Non-goals

- Removing the old reads (relais, greengate, greenharness-quiet, and
  greenpiste's own old-header and old-schema reads): the programme census,
  with greengate's 3a-2 Phase 6.
- The binary alias `amont-agent` and the `amont-agent-*` release archives:
  the census.
- The local checkout directory `~/Developer/Perso/amont-agent` and the
  repository name in plan markers (`repos=amont-agent`): with the other
  checkouts at the census.
- Other repositories' vendored `greengate.conf` pack lines naming
  `amont-agent suppressions`: the fleet sweep (step 4).
- Old plan records under `docs/plans/` in any repository: history.

## Code review
- amont-agent: backend, lang:rust, lang:typescript, unix, tui, architect
- relais: backend, lang:rust (unix, tui, architect left out: two readers gain a fallback, no output or boundary change; lang:typescript left out: no TypeScript file)
- amont: backend, lang:rust (unix, tui, architect left out: two readers gain a fallback and one message names greenpiste; lang:typescript left out: no TypeScript file)
- grh: backend, unix (lang:rust, lang:typescript, tui, architect left out: one shell allowance and two pins)
- greenharness-skills: none — markdown only
- dotfiles: unix (backend, lang:*, architect left out: a markdown template and one shell script's paths)
- decisions: none — markdown only

## Behaviour

- **One names module.** `greenpiste-guard::names` (it exists since 3b-1, holding `DATA_DIR` and `RECORD_TOOL`) is changed to hold every data name:
  `DATA_DIR = "greenpiste"`, `CONFIG_PREFIX = "greenpiste."`,
  `ENV_PREFIX = "GREENPISTE_"`, the header and schema strings
  (`greenpiste-v1`, `greenpiste-cases-v1`, `greenpiste-review-*-v1`,
  `greenpiste-impl-review-outcome-v1`), `RECORD_TOOL`, the provenance
  producer word and the temp prefix; and, in a `names::old` submodule, the
  `amont-agent` forms the reads and the migration use. Call sites build
  from them (`gitconfig.rs` `PREFIX`, `stance.rs` `ENV_OFF`, the env reads
  in `plan_review.rs`, `pin_check.rs`, `shipped.rs`, `review_brief.rs`,
  `hook.rs`, `journal.rs`, `quiet.rs`). `tests/names.rs` fails on a literal
  `amont-agent`, `amont.agent.` or `AMONT_AGENT_` outside `names.rs`,
  `migrate.rs` and the corpus header fallback.
- **Written: only new names.** Env and config are read under the new names
  only (an old key or variable is ignored, and `doctor` lists each one it
  finds with the command that moves it). Every row, header, record field
  and temp file is written new.
- **Read: both, where old data stays.** The journal reader accepts
  `greenpiste-v1` and `amont-agent-v1` (old rotations stay readable). The
  outcome-ledger readers accept both schema strings per row. The corpus
  loader accepts both cases headers (committed files, until the census).
  The provenance `producer` word is not read by greenpiste.
- **First-run migration of this machine.** Normal 5.0 entry points, hooks
  included, run `migrate::machine()` before they read or write data.
  Dispatch `migrate` (including `--reverse`) directly to its requested
  migration direction, without a forward preflight; `doctor` inspects
  incomplete migration without triggering it. Help, version and dry-run
  commands never migrate. Test these exceptions with a deliberately
  failing migration step, asserting recovery remains reachable and
  informational/dry-run commands leave files unchanged. Automatic migration
  holds `greenharness_slots`' lock (as greengate's `migrate.rs`) on a file
  in the parent of the config directory, which the migration never moves.
  A done-marker skips the work afterwards; a reversed-marker, checked
  again under the lock (greengate `migrate.rs:371,379-389`), stops a 5.0
  run from migrating forward again after `--reverse`. In a hook the lock is
  tried with zero wait: when it is held or a step fails, the hook goes on
  without blocking or refusing, reading and writing whichever root exists
  (the new one once moved, else the old one; data written there moves with
  it). Explicit commands and `install` wait for the lock (capped, said
  once) and refuse with what to fix. Each step is idempotent and checked:
  1. `~/.claude/amont-agent` is renamed to `~/.claude/greenpiste`
     (rename(2), same filesystem; a symlink is left at the old path); a
     second root, `<state>/amont-agent`, the same way;
  2. every global `amont.agent.*` key is mapped by stripping `amont.agent.`
     and prepending `greenpiste.` (subsection kept as written: names are
     read from the actual source files, resolving GIT_CONFIG_GLOBAL,
     Git's global config locations and their include/includeIf graph;
     preserve subsection spelling, include conditions and file precedence.
     Rename keys in their source files rather than flattening conditional
     values into ~/.gitconfig. Inventory system-scope old keys too: any
     unsupported or unwritable origin prevents a completion marker and
     explicit migration reports the file and required repair; hooks retain
     the specified fall-through behaviour, and until the done-marker exists they read each config key from the old `amont.agent.*` name when its new name is absent (so `denyAgents` keeps applying); a failed step writes a failed-marker next to the lock, hooks stop retrying, and `doctor` names it. Test: with an unwritable origin, a hook call still applies `denyAgents` from the old key, and 50 hook calls make one migration attempt. Reverse uses the same origins.
     Test alternate global files and active/inactive conditional includes,
     comparing effective values before, after and after reverse. A
     lowercased variable such as `denyagents` is git-equivalent); every value
     (`--get-all`) is copied unless already present, the copy is verified,
     and only then are the old keys removed, so an interruption between
     copy and removal leaves the old keys intact and a re-run adds nothing;
  3. the plugin registration in `settings.json` moves from
     `clear-relay@amont-agent` to `clear-relay@greenpiste` (marketplace id,
     path, plugin key and `stateDir`), through the same writer
     `plugin_install.rs` uses, with the formatting guard; the placed files
     (incl. `.claude-plugin/marketplace.json`, whose `name` is the marketplace id)
     are rewritten, and `--reverse` rewrites them from `names::old`.
  A step that cannot finish leaves the old names untouched and refuses with
  what to fix. `greenpiste migrate --reverse` runs the steps backwards;
  local `amont.agent.*` keys in a repository (none on this machine) are
  moved by `greenpiste migrate` in that repository.
- **The TypeScript plugin** already reads `stateDir` (`register.ts:253`, written by
  `settings.rs:200`); only the empty-`stateDir` fallback (`register.ts:102`) and the
  `plugin.json` description name `amont-agent`. The fallback becomes
  `$XDG_STATE_HOME/greenpiste`, else `~/.local/state/greenpiste` (as `state_root()`), and
  `register.test.ts` gains a no-options case asserting that log path.
- **relais and greengate read both** before 5.0 ships: relais
  `review_outcome.rs` reads `<config>/greenpiste/plan-review/outcomes.jsonl`
  first and the old path when the new is absent, accepts both schemas, and
  reads `GREENPISTE_PLAN_ROOT` then `AMONT_AGENT_PLAN_ROOT`; relais
  `quiet.rs` and greengate's `greenharness-quiet` skip both temp prefixes
  and accept both `tool` values; greengate `attest.rs` reads
  `<config>/greenpiste/provenance` then the old directory, and prints
  "greenpiste" in its message.
- **Installers** read `GREENPISTE_BIN_DIR` / `GREENPISTE_VERSION` first and
  the `AMONT_AGENT_*` names when the new are unset, with one warning naming
  the new one.
- **Written instructions say the new names:** greenharness-skills
  (`worktree-task`, `plan-review`, `retro`, `browser-check`), dotfiles
  (`dot_claude/CLAUDE.md.tmpl`, `retro-weekly.sh`), grh `scripts/e2e.sh`
  (leftover allowance `.claude/greenpiste/`).
- **Release order.** relais and greengate releases that read both are
  installed here first; then greenpiste 5.0.0 is released and installed and
  this machine migrates (its first command); then grh, skills and dotfiles.
  Rollback before migration: reinstall 4.0.1. After: `greenpiste migrate
  --reverse` (it also rewrites the journal header and new-schema rows back, tested by a 4.0.1-style read), then 4.0.1.

## Phases

- [x] Phase 1 — readers read both: `relais/crates/relais/src/review_outcome.rs`,
  `relais/crates/relais/src/quiet.rs`,
  `amont/crates/greengate-runtime/src/attest.rs`,
  `amont/crates/greenharness-quiet/src/lib.rs`; relais and greengate
  released and installed here.
- [x] Phase 2 — greenpiste writes the new names:
  `amont-agent/crates/greenpiste-guard/src/names.rs`,
  `amont-agent/crates/greenpiste-guard/src/gitconfig.rs`,
  `amont-agent/crates/greenpiste-guard/src/stance.rs`,
  `amont-agent/crates/greenpiste-guard/src/shadow.rs`,
  `amont-agent/crates/greenpiste-guard/src/journal.rs`,
  `amont-agent/crates/greenpiste-guard/src/corpus.rs`,
  `amont-agent/crates/greenpiste-guard/src/quiet.rs`,
  `amont-agent/crates/greenpiste-guard/src/doctor.rs`,
  `amont-agent/crates/greenpiste-guard/src/settings.rs`,
  `amont-agent/crates/greenpiste-process/src/review_outcomes.rs`,
  `amont-agent/crates/greenpiste-process/src/plan_apply.rs`,
  `amont-agent/crates/greenpiste-process/src/plan_review.rs`,
  `amont-agent/crates/greenpiste-process/src/pin_check.rs`,
  `amont-agent/crates/greenpiste-process/src/provenance.rs`,
  `amont-agent/crates/greenpiste-process/src/candidates_block.rs`,
  `amont-agent/src/hook.rs`, `amont-agent/src/plugin_install.rs`,
  `amont-agent/tests/names.rs`, `amont-agent/tests/plugin_install.rs` (line 9 `clear-relay@amont-agent`), and the rest of the env and config call
  sites (`rg -l 'AMONT_AGENT_|amont\.agent\.' crates src`).
- [x] Phase 3 — migration and the plugin:
  `amont-agent/crates/greenpiste-guard/src/migrate.rs` (new),
  `amont-agent/src/main.rs` (`migrate`, `migrate --reverse`),
  `amont-agent/plugin/clear-relay/hooks/register.ts`,
  `amont-agent/plugin/clear-relay/.claude-plugin/plugin.json`,
  `amont-agent/plugin/clear-relay/hooks/register.test.ts`,
  `amont-agent/install/install.sh`, `amont-agent/install/install.ps1`,
  `amont-agent/tests/migrate.rs` (new).
- [ ] Phase 4 — release 5.0.0 and migrate this machine:
  `amont-agent/CHANGELOG.md`, `amont-agent/docs/configuration.md`,
  `amont-agent/docs/install.md`, `amont-agent/Cargo.toml`,
  `amont-agent/greenharness.toml`; released, installed here, first
  command migrates.
- [ ] Phase 5 — written instructions: `greenharness-skills/parts/workflow/skills/worktree-task/SKILL.md`,
  `greenharness-skills/parts/workflow/skills/plan-review/SKILL.md`,
  `greenharness-skills/parts/workflow/skills/retro/SKILL.md`,
  `greenharness-skills/parts/workflow/agents/browser-check.md`,
  `dotfiles/dot_claude/CLAUDE.md.tmpl`,
  `dotfiles/dot_claude/tools/retro-weekly/executable_retro-weekly.sh`,
  `grh/scripts/e2e.sh`, `grh/catalog.toml` (skills pin), `grh/manifests/greenpiste.toml`
  (5.0.0 pin); skills and grh released.
- [ ] Phase 6 — record: `decisions/docs/programmes/gate-guardrails/map.md`
  (3b-2 done; the census list gains every old read this plan keeps).

## Verification

- Phase 1: relais and greengate tests read a ledger, a provenance dir and a
  temp file under each name; with both present the new wins; released
  versions installed here answer `--version`.
- Phase 2: `cargo test --workspace`; the names test fails on a planted
  `AMONT_AGENT_X` literal; with `CLAUDE_CONFIG_DIR` set to a scratch dir,
  a hook run writes `journal.log` under `greenpiste/` with header
  `greenpiste-v1`, and an old `amont-agent-v1` rotation is still read
  (`greenpiste status` counts its rows); an outcome row of each old schema
  is read.
- Phase 3: tests set HOME, XDG_*, CLAUDE_CONFIG_DIR, GIT_CONFIG_GLOBAL and GIT_CONFIG_NOSYSTEM and assert the live ~/.gitconfig is unchanged; on a scratch HOME holding a copy of a real old tree (old dirs,
  30 global keys incl. a lowercased one, a settings.json with
  `clear-relay@amont-agent`): `greenpiste migrate` moves both roots and
  leaves symlinks, moves every key (`git config --global --list` before
  and after compared), moves the plugin registration; a second run changes
  nothing; `migrate --reverse` restores the old tree (`diff -r` against the
  copy); a held lock makes an explicit `migrate` refuse with what to fix
  and a hook run go on without migrating; a `kill -9` after step 2 and a
  re-run add no duplicate global value and finish step 3; after
  `--reverse`, a hook run moves nothing (`diff -r` still clean); a hook
  run concurrent with the step-1 rename succeeds; the placed
  `marketplace.json` names the current marketplace in each direction;
  `register.test.ts` passes, including the no-options case; `greenpiste
  doctor` green means it lists no old key, root or registration.
- Phase 4: the release run concludes `success` (crates, tap, verify-brew);
  on this machine the first command after `brew upgrade` migrates:
  `~/.claude/greenpiste/journal.log` exists, `~/.claude/amont-agent` is a
  symlink, `git config --global --get-regexp '^amont\.agent\.'` is empty
  and `^greenpiste\.` lists 30, settings.json names
  `clear-relay@greenpiste`, `greenpiste doctor` is green, and relais's
  plan-review stats and greengate's provenance push read the moved data.
- Phase 5: skills and grh CI green; `rg -n 'amont-agent|amont\.agent|AMONT_AGENT'`
  over the changed skill and dotfile files lists none.

## Rollback

- Phase 4: `greenpiste migrate --reverse` (rows and headers written by 5.0 since migration are rewritten to the old schema, or listed as lost before 4.0.1 runs), then `brew` back to 4.0.1 (a tap
  commit pinning it), or install.sh with 4.0.1.
- Phases 1–3: unreleased code, or releases that only add reads; revert.

## Outcome

On this machine nothing greenpiste writes says `amont-agent`, and the old
reads left are the ones the census list names.

## Decision log

- 2026-11-14 — ADR-0068's hard-rename path, as greengate's 3a-2: greenpiste
  has no outside users; old rows and committed corpus files are read, never
  rewritten.
- 2026-11-14 — major version 5.0.0: env and config names change.
- 2026-11-15 — the migration lock is greenpiste's own `flock` helper, moved to `greenpiste-guard::lock`, not `greenharness-slots`': greenpiste takes no `greenharness-*` crate (every part stays optional), and the lock only guards greenpiste's own migration.
- 2026-11-15 — the greengate Phase 1 reviews found that a provenance payload still in the old directory was skipped once the new root existed; greengate now reads per payload, so Phase 3 needs no ordering constraint on provenance.

<!-- panel: repos=amont-agent,relais,amont,grh,greenharness-skills,dotfiles,decisions adds=contract reviewers=backend,lang:rust,lang:typescript body-sha=2f09c81bd251 -->
