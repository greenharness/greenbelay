---
status: done
branch: feat/greenpiste-becomes-greenbelay
repos: [amont-agent, relais, amont, grh, greenharness-skills, decisions, dotfiles]
adrs: [ADR-0064, ADR-0068]
plan-id: p-01M7JVP8V0JEC3J525R31J0SXY
---
# greenpiste becomes greenbelay

## Review panel

👉 **Decide:** none — approve if greenpiste becomes greenbelay by sed in 7 repos, this Mac switched by one rehearsed script.
📍 7 repos · Phase 0 merged (ADR-0071, decisions#213); Phase 1 reviewed · next: greenbelay 6.0.0 PR and release. Panel: backend, lang:rust, lang:python, platform, tui, unix, architect, po (opus-5-5), codex (gpt).
**Changed by review:** relais's `amont-agent` lookups renamed; switch order rebuilt (pin, atomic local copy, settings sed, no evidence deleted); ADR replaces the rename rule first.
📄 Full reviews: [2026-11-16-greenpiste-becomes-greenbelay.reviews.md](2026-11-16-greenpiste-becomes-greenbelay.reviews.md)
**Verdicts:** round 1 approve-with-changes 5, rework 1; final bind approve 3, approve-with-changes 5; codex rework (3 applied); 0 rework left; 17 carried into implementation (listed below).

## Picture

```
         today: greenpiste 5.1.0       after: greenbelay 6.0.0
         -----------------------       -----------------------
 repo    greenharness/greenpiste       greenharness/greenbelay
 crates  greenpiste(-guard,-process)   greenbelay(-guard,-process)
 binary  greenpiste + amont-agent      greenbelay, no copy
 tap     Formula/greenpiste.rb         Formula/greenbelay.rb
 data    ~/.claude/greenpiste          ~/.claude/greenbelay
         ~/.local/state/greenpiste     ~/.local/state/greenbelay
         30 greenpiste.* git keys      30 greenbelay.* git keys
         greenpiste-* row headers      greenbelay-* (data sed)
 readers relais, greengate, grh,       greenbelay names only
         skills, dotfiles
```

The rename is text: every live `greenpiste` becomes `greenbelay`, in the
code and in this machine's data, by sed. No alias, no migration code: the
only machine is this one, switched by a one-time script.

## Context

The person chose the climbing family for the brand (2026-11-16): `piste` is
the one name that is skiing only, and a belay is what catches a fall live,
which is what the hook does. They asked for the rename by sed, fast, with no
aliases, because there are no users besides this machine. `greenbelay` is
free on crates.io (greenbelay, -guard, -process), npm, GitHub
(`greenharness/greenbelay`) and Homebrew core (checked 2026-11-16).

Found (2026-11-16): about 2,300 live hits in 7 repositories, 1,895 of them in
greenpiste's own repo; names are centralised in
`crates/greenpiste-guard/src/names.rs` (`PRODUCT`, `BIN`, `DATA_DIR`,
`CONFIG_PREFIX`, `ENV_PREFIX`, schemas, markers, `bin!`, `config_key!`,
`env_var!`). Other tools read greenpiste's data: relais
(`review_outcome.rs`: `greenpiste/plan-review/outcomes.jsonl`, the
`greenpiste-review-*` schemas, `GREENPISTE_PLAN_ROOT`), greengate
(`attest.rs` provenance dir and owner, `greenharness-quiet` temp prefix and
tool value, `ci.yaml:72` fetches `greenharness/greenpiste` contents), grh
(part name, pin, manifest, goldens), the skills (commands, paths, keys) and
chezmoi (settings template, `retro-weekly.sh`, Brewfile). No other session
holds an open branch on greenpiste's repo (board, 2026-11-16).

## Goal

After this plan, nothing live names greenpiste: the binary, crates,
formula, repo, data directories, git keys, env vars, row headers, plugin id
and every reader are greenbelay, this machine runs on greenbelay with its
history readable, and greenpiste is left only in history (CHANGELOGs, landed
plans, ADR records, programme maps).

## Non-goals

- Aliases or migration code for greenpiste names: no `greenpiste` binary copy
  shipped, no fallback reader, no `names::old` second hop. (This machine keeps
  a local `/usr/local/bin/greenpiste` copy of greenbelay only until the
  sessions running at the switch have restarted; it is not shipped.)
- The `amont_agent = "required"` key in about 40 repos' `relais.toml`: it keeps
  its name (a fleet sweep for the census); relais maps it to the greenbelay
  binary.
- The `amont-agent` era: `names::old` and its migration stay as they are
  (the census of 2026-12-13 removes them); this plan only drops the
  `amont-agent` binary copy.
- Rewriting history: CHANGELOG entries, `docs/plans/`, ADR records and
  programme maps keep the name they were written with.
- Yanking the published `greenpiste` crates (left as they are, their README
  says where the tool went).
- The other renames (relais, aval, adr-lint, attest).

## Code review
- amont-agent: backend, lang:rust, lang:python, platform, unix (tui left out: no output changes but the name)
- relais: backend, lang:rust (platform, unix, tui left out: a path, a schema and an env name; no CLI or CI change)
- amont: backend, lang:rust, platform (unix, tui left out: two readers and a CI path, no CLI change)
- grh: backend, lang:rust (platform, unix, tui left out: a pin, a part name, goldens and their Rust tests; no CLI or CI change)
- greenharness-skills: none — markdown only
- decisions: none — markdown only
- dotfiles: none — templates and a script's paths, checked by the Phase 4 run

## Behaviour

- **The sed**, per repository, over tracked files except history (`CHANGELOG.md`,
  `docs/plans/`, `docs/adr/`, `docs/programmes/`, `.adr/`, vendored packs):
  `greenpiste`→`greenbelay`, `Greenpiste`→`Greenbelay`,
  `GREENPISTE`→`GREENBELAY` (the repo URL follows from the first). Paths move
  with `git mv`: `crates/greenpiste-guard`, `crates/greenpiste-process`,
  `packaging/homebrew/greenpiste.rb`, `grh/manifests/greenpiste.toml`. The
  hand edits below run AFTER the sed. Then `git grep -i greenpiste` outside
  history lists only an allowlist, named in each PR: the tap step's rename key
  and its `rm Formula/greenpiste.rb`, and the release upgrade test's
  `greenpiste` install; everywhere else (grh included) it is 0.
- **What the sed cannot do, done by hand:**
  - greenbelay's release job: the tap step writes `formula_renames.json`
    `{"greenpiste": "greenbelay"}` (the `amont-agent` entry dropped) and
    removes `Formula/greenpiste.rb`; every `amont-agent` check goes (install
    smoke tests and `cmp` at `release.yaml:166-217,349-378,404-405`,
    `verify-brew` at `:762-763`), and the upgrade test (`:781-815`) proves
    greenpiste → greenbelay: install greenpiste from the parent of the tap
    commit that deletes it, `brew trust --formula fredericrous/tap/greenbelay`,
    `brew upgrade fredericrous/tap/greenpiste`, check `greenbelay --version`;
  - the `amont-agent` binary copy goes: the formula's `cp`/`bin.install` and
    its `amont-agent --version` test, the release's alias archives,
    `Makefile`, `install/install.{sh,ps1}`, and `greenharness.toml`
    `bin_aliases`; `scripts/release-manifest.py` refuses any `bin_aliases`
    (one name per tool, `harness.bundle-names`), and
    `test_release_manifest.py:66,117-125` tests that refusal;
  - the formula's caveats say no old-name binary is installed and that hooks
    calling the old path fail until `greenbelay install --write`;
  - relais looks up the binary `amont-agent` (`tooling.rs:75-81,159,264`,
    `verify/mod.rs:3530-3531`, `doctor/mod.rs:228,1446,2761-2779`): each
    becomes `greenbelay`, tests included;
  - greengate's messages that name a greenpiste version that greenbelay never
    had (`attest.rs:363,452,564`) are reworded;
  - skills: a version threshold for greenpiste older than 6.0.0 ("older than
    3.22.0") is removed, not renamed: every greenbelay is 6.0.0 or later;
  - grh: the pin and `manifests/greenbelay.toml` come from the greenbelay
    6.0.0 release (`cargo run --example pins -- pin greenbelay
    greenharness/greenbelay v6.0.0`), not from sed; the part keeps
    `aliases = ["guardrails"]` only;
  - chezmoi's `settings.json.tmpl` runs `/usr/local/bin/amont-agent` (drift
    from the live file): it becomes `/usr/local/bin/greenbelay`.
- **No upgrade before the switch:** `brew pin greenpiste` before greenbelay's
  release (checked: `brew upgrade --dry-run` does not list it), so no ordinary
  upgrade swaps out the binary the 17 hooks call before Phase 4.
- **This machine's switch**, `~/.local/state/greenbelay-switch.sh` (bash,
  `set -euo pipefail`), rehearsed first with `HOME`, `CLAUDE_CONFIG_DIR`,
  `XDG_STATE_HOME` and `GIT_CONFIG_GLOBAL` pointed at a copy, then run once.
  Each step is checked and safe to run again; a failed step prints the step,
  its cause and `sh ~/.local/state/greenbelay-switch.undo.sh`, and exits
  non-zero. Before acting it writes the undo script and copies
  `~/.claude/settings.json`. No greenbelay or grh command runs before step 2.
  0. refuse if `~/.claude/greenbelay`, `~/.local/state/greenbelay` or
     `~/.claude/greenbelay-migrated` exists;
  1. remove the `amont-agent` symlinks `~/.claude/amont-agent` and
     `~/.local/state/amont-agent` (with them gone, greenbelay's kept
     `names::old` migration finds no old root);
  2. `mv ~/.claude/greenpiste ~/.claude/greenbelay` and
     `mv ~/.local/state/greenpiste ~/.local/state/greenbelay` (plain `mv`:
     macOS `/bin/mv` has no `-T`; step 0 guarantees the targets are absent),
     then rename
     the markers `~/.claude/greenpiste-migrat*` to `greenbelay-*`;
  3. sed `greenpiste-` → `greenbelay-` in the row headers and schemas of
     `journal.log*` and `*.jsonl` under `~/.claude/greenbelay` (attestations
     and binaries untouched), `greenpiste` → `greenbelay` in the marketplace
     manifest under `~/.local/state/greenbelay`, and the part name in grh's
     receipt;
  4. rename the git keys: one `git config --rename-section` per distinct
     section of `git config --name-only --get-regexp '^greenpiste\.'` (24
     `[greenpiste "<rule>"]` subsections and the bare `[greenpiste]`, global
     and the one repo-local); fail if a `greenpiste` section remains;
  5. `brew unpin greenpiste`, `brew trust --formula fredericrous/tap/greenbelay`,
     `brew upgrade fredericrous/tap/greenpiste` (the rename moves the keg;
     `brew list greenpiste` fails after), then at once
     `cp "$(brew --prefix)/bin/greenbelay" /usr/local/bin/.greenpiste.new`,
     one `/usr/local/bin/.greenpiste.new --version`, then
     `mv /usr/local/bin/.greenpiste.new /usr/local/bin/greenpiste` (an atomic
     rename in one directory: it replaces a leftover brew link rather than
     writing through it, and no hook runs a half-written file; checked
     regular file, not a link; a copy, never a symlink, as the new ADR says; machine-local: the
     sessions running now keep their startup hook snapshot, and it runs
     greenbelay for them); then the script itself rewrites
     `~/.claude/settings.json` by sed (`/usr/local/bin/greenpiste` →
     `/usr/local/bin/greenbelay`, `clear-relay@greenpiste` →
     `clear-relay@greenbelay`, the marketplace name and path): greenbelay's
     installer recognises only its own and the `amont-agent` command as its
     hooks, so `install --write` over the old entries would add a second
     handler beside each; after the sed, `greenbelay install` (no `--write`)
     reports nothing to change and each event/matcher has exactly one
     greenbelay handler;
  6. a hook that fired during steps 2–5 may have left a stray
     `~/.claude/greenpiste` or `~/.local/state/greenpiste`: append its
     `journal.log*` and `*.jsonl` rows into greenbelay's with the step-3 sed
     applied, and move every other file to the same path under greenbelay
     when that path is free; a file whose path is taken stays in the stray
     directory, which is kept and named in the script's last line (never
     deleted while it holds anything);
  7. `claude plugin uninstall clear-relay@greenpiste` and
     `claude plugin marketplace remove greenpiste` (doctor's old side is
     `amont-agent`, so it never names greenpiste), then the commands
     `greenbelay doctor` names for the new side.
  Then relais, greengate and grh are upgraded, and `grh install workflow`.
  The `/usr/local/bin/greenpiste` copy is removed the next day, once every
  session running at the switch has restarted.

## Phases

- [x] Phase 0 — the decision, before any code: a new ADR in
  `decisions/docs/adr/` with `decisions/.adr.yaml` that replaces ADR-0064's
  `harness.bundle-names` (greenbelay names the part) and the rename clause of
  ADR-0068's `harness.renames` (a tool with no outside users may hard-rename
  at the person's choice: no shipped binary copy, no old-path state symlink;
  this machine keeps a local binary copy for a day), the person's choice of
  2026-11-16. ADR-0068's old-signed-notes sunset stays bound to its key, and
  the census of 2026-12-13 for `names::old` stays.
- [x] Phase 1 — greenbelay 6.0.0: first rename the GitHub repo
  `greenharness/greenpiste` → `greenharness/greenbelay` (`gh api -X PATCH`,
  before any reader's CI fetches the new path), then the sed and the hand
  edits in
  `amont-agent/crates/greenpiste-guard/src/names.rs` and every file the sed
  reaches, `amont-agent/.github/workflows/release.yaml`,
  `amont-agent/packaging/homebrew/greenpiste.rb`,
  `amont-agent/greenharness.toml`, `amont-agent/scripts/release-manifest.py`,
  `amont-agent/Makefile`, `amont-agent/install/install.sh`,
  `amont-agent/Cargo.toml`, `amont-agent/CHANGELOG.md`; `brew pin greenpiste`
  before the release; PR, release, crates and tap published.
- [x] Phase 2 — readers: `relais/crates/relais/src/review_outcome.rs`,
  `relais/crates/relais/src/doctor/mod.rs`, `relais/crates/relais/src/cli/mod.rs`,
  `relais/crates/relais/src/tooling.rs`, `relais/crates/relais/src/verify/mod.rs`,
  `relais/README.md`, `relais/CHANGELOG.md`;
  `amont/crates/greengate-runtime/src/attest.rs`,
  `amont/crates/greenharness-quiet/src/lib.rs`,
  `amont/.github/workflows/ci.yaml`, `amont/.github/workflows/release.yaml`,
  `amont/CHANGELOG.md`; both released.
- [x] Phase 3 — grh and skills: `grh/catalog.toml`,
  `grh/manifests/greenpiste.toml`, `grh/src/catalog.rs`, `grh/tests/cli.rs`,
  `grh/tests/no_part_names.rs`, `grh/tests/fixtures/audit/expected-new.json`,
  `grh/scripts/e2e.sh`, `grh/CHANGELOG.md`;
  `greenharness-skills/parts/workflow/skills/plan-review/SKILL.md` and the
  other `parts/` files the sed reaches; skills released, grh pins it and
  greenbelay 6.0.0, released.
- [x] Phase 4 — switch this machine: rehearse, then run the switch script (it
  installs greenbelay itself); then upgrade relais, greengate and grh and
  `grh install workflow`;
  `dotfiles/dot_claude/settings.json.tmpl`,
  `dotfiles/dot_claude/tools/retro-weekly/executable_retro-weekly.sh`,
  `dotfiles/private_dot_config/Brewfile` follow the live files (PR).
- [x] Phase 5 — record it: `decisions/docs/programmes/gate-guardrails/map.md`.

## Verification

- Phase 1: `cargo test --workspace --features test-hooks`, `scripts/leak-hunt.sh`
  clean on every target; `git grep -i greenpiste` outside history shows only the allowlist; the
  release run concludes `success`; crates.io lists greenbelay,
  greenbelay-guard, greenbelay-process 6.0.0; the tap has
  `Formula/greenbelay.rb` and `formula_renames.json` maps greenpiste.
- Phase 2: relais and greengate test suites green; each release run
  `success`; greengate's CI fetch of `greenharness/greenbelay` passes;
  `relais doctor` in amont-agent with no `amont-agent` on PATH finds the
  integration.
- Phase 3: grh `make pins-check` green; `grh status` lists greenbelay;
  skills release run `success`.
- Phase 4: the rehearsal on the copy passes every check below first. After
  the script: `greenbelay doctor` green (no registry finding, migration
  section clean), `~/.claude/greenpiste` and `~/.claude/greenbelay/greenpiste`
  absent, `claude plugin list` names no greenpiste, journal row count equal
  before and after, no
  `greenpiste.*` git key left, `grep -c greenpiste ~/.claude/settings.json`
  = 0, `greenbelay status` reads the journal rows written before the switch,
  `relais review-outcome` (or the time-to-done stats) reads the plan-review
  outcomes written before it, a Bash call in a session started after the switch is judged by greenbelay
  (a refused command names `greenbelay/<rule>`), and one in this session too,
  through the local copy.
- Phase 0: `aval resolve harness.bundle-names` and `aval resolve
  harness.renames` in `decisions` name the new record, with no contradiction.

## Rollback

- Before Phase 4: nothing on this machine changed; revert the PRs, rename the
  repo back (`gh api -X PATCH`), push a tap commit restoring
  `Formula/greenpiste.rb` and removing the rename entry, and `brew unpin
  greenpiste`; the greenbelay crates stay published.
- Phase 4: the undo script reverses each step: the two `mv`s back, the
  step-3 sed reversed (`greenbelay-` → `greenpiste-`, rows written after the
  switch included), the git sections back, `~/.claude/settings.json`
  restored from its copy, the step-7 plugin commands reversed
  (`clear-relay@greenpiste` and its marketplace registered again), relais,
  greengate and grh reinstalled at the versions the script recorded before
  Phase 4's upgrades (their greenbelay readers do not read greenpiste data),
  the `/usr/local/bin/greenpiste` copy removed with
  `rm` and `brew uninstall greenbelay` (so nothing is written through onto
  greenbelay's binary), then greenpiste 5.1.0 reinstalled from its GitHub
  release archive (the tap no longer has its formula).

## Implementation review

- greenbelay (Phase 1): implementation-review approve after 2 deltas (the committed `.pyc` fixed; all 33 candidates fine); backend approve (caveat names the real fix; upgrade test checks amont-agent too); lang:rust, lang:python, platform approve; unix approve with installer `AMONT_AGENT_*` fallbacks, stderr colour and old comments deliberate (amont-agent era, the census owns them).
- Deliberate: Apple-silicon prefix in the caveat (the only machine is Intel `/usr/local`); malformed-JSON traceback in the tap heredoc (the workflow writes the file).
- Done before the tag (platform high): `/usr/local/bin/greenpiste` replaced by a regular-file copy of 5.1.0 (`cmp` equal), so a brew rename migration cannot remove the hook path before the switch; `brew pin greenpiste` set.

## Outcome

Observed before the push (2026-11-16):

- Phase 0: ADR-0071 merged (decisions#213, 419c7418); `aval resolve harness.bundle-names` and `harness.renames` return ADR-0071, no contradiction.
- Phase 1: GitHub repo renamed `greenharness/greenpiste` → `greenharness/greenbelay` (old name resolves); `git grep -i greenpiste` outside history shows only the allowlist (release.yaml's tap rename and upgrade test, the formula's caveat and history line, test fixtures); `cargo test --workspace --features test-hooks --no-fail-fast` 45 targets ok; `scripts/leak-hunt.sh` 43 ok; `python3 -m unittest discover -s scripts` 22 ok.

- Phase 1: greenbelay 6.0.0 released (#276, tag v6.0.0 = 747fd78; release run success; crates greenbelay, -guard, -process 6.0.0; the tap has `Formula/greenbelay.rb` and `formula_renames.json` maps greenpiste).
- Phase 2: relais 0.67.0 (#484/#485) and greengate 4.10.0 released, each release run success.
- Phase 3: skills 0.12.0 and grh 0.8.0 released; `grh status` lists greenbelay ✓.
- Phase 4 (2026-11-17): the switch script was rehearsed on a copy, with a forced stop and a resume, and every check passed. It then ran here, steps 0–7 ok, rc=0. Checks after the switch:
  - `~/.claude/greenpiste`, `~/.local/state/greenpiste` and `~/.claude/greenbelay/greenpiste` are absent;
  - settings.json names no greenpiste, and all 17 hooks run `/usr/local/bin/greenbelay`;
  - no `greenpiste.*` git key is left, and there are 25 `greenbelay` sections;
  - the journal went from 1426 to 1430 rows, and the last row from before the switch is present;
  - doctor is green (migration: nothing left);
  - `claude plugin list` shows `clear-relay@greenbelay` only;
  - `/usr/local/bin/greenpiste` is a regular-file copy of 6.0.0;
  - the outcomes file holds `amont-agent-review-*` and `greenbelay-review-*` rows, which relais reads, and 0 rows named greenpiste.
- Dotfiles: dotfiles#66 merged. The templates follow the live files, and `CLAUDE.md` plus `retro-weekly.sh` were fixed: the switch broke them, since they called `amont-agent` and read `~/.claude/greenpiste`.
- Found after the switch: three repos (this one, decisions and grh) vendored the `fredericrous/amont-agent` greengate pack, which runs `amont-agent suppressions`, so their commit check could not run. They now vendor `greenharness/greenbelay` at v6.0.0.
- Phase 5: Step 3c is recorded in the decisions gate-guardrails map.
- Open, dated in the person's notes:
  - remove the `/usr/local/bin/greenpiste` copy once no `claude` process older than the switch remains, then check that `~/.local/state/greenpiste` stays absent;
  - the 7-day measure of judged Bash calls per day against greenpiste's 7-day median, due 2026-11-24.

## Decision log

- 2026-11-16 — climbing names for the brand family begin with greenbelay;
  sed, no aliases (the person).
- 2026-11-16 — move this machine's data and sed its headers, rather than
  start fresh: the git keys hold 30 rule stances, and the plan-review
  outcomes feed the time-to-done programme.
- 2026-11-16 — running sessions keep their startup hook snapshot: a local
  copy at `/usr/local/bin/greenpiste` of greenbelay for a day, rather than
  closing every session before the switch (panel round 1, unix #1; a copy, not
  a symlink, round 2 architect #2).



<!-- panel: repos=amont-agent,relais,amont,grh,greenharness-skills,decisions,dotfiles adds=contract reviewers=architect,backend,lang:python,lang:rust,platform,po,tui,unix body-sha=3b94ca630036 -->
