---
status: active
branch: fix/greenpiste-plugin-registry
repos: [amont-agent, grh]
adrs: [ADR-0068]
plan-id: p-01M7H8RWZF2TBZTBW0BCZQ5ANA
---
# greenpiste tells Claude Code's registry what moved

## Review panel

👉 **Decide:** none — approve if greenpiste should name the `claude plugin` commands that fix Claude Code's registry, and never run them itself.
📍 amont-agent, grh · Phases 1–2 built · next: implementation review, then Phase 3 (release 5.0.1). Panel: backend, lang:rust, tui, unix (opus-5-5), codex (gpt).
**Changed by review:** doctor prints one command per line (its sanitiser escaped `\n`); the leak hunt catches a write into the canary and passes features per package; previews and already-migrated machines are judged against the side they will write.
📄 Full reviews: [the sidecar](2026-11-16-greenpiste-tells-claude-codes-registry.reviews.md).
**Verdicts:** round 1 approve-with-changes 4; round 2 approve 1, approve-with-changes 3; Codex rework (2 high applied); final backend bind approve-with-changes; 0 rework left; 9 carried into implementation.

## Picture

```
 after `migrate` (5.0.0)                  after this plan (5.0.1)
 -----------------------                  -----------------------
 settings.json   clear-relay@greenpiste   same
 Claude registry clear-relay@amont-agent  same files, never written
   (installed_plugins.json,               by greenpiste; migrate,
    known_marketplaces.json)              install and doctor name
 the plugin      disabled, silently       the stale entry and the
                                          `claude plugin` commands
 leak hunt       tests/*.rs only          also each crate's unit tests
```

greenpiste reads Claude Code's plugin registry and says what to run; it never
writes it.

## Context

greenpiste 5.0.0's migration renamed the clear-relay plugin in
`~/.claude/settings.json` from `clear-relay@amont-agent` to
`clear-relay@greenpiste`. Claude Code's own registry
(`~/.claude/plugins/known_marketplaces.json`, `installed_plugins.json`) was
not touched, so on the first machine migrated (2026-11-15) the plugin stayed
disabled until two `claude plugin` commands were run by hand, while `doctor`
was green. The data-names plan recorded this, and that
`scripts/leak-hunt.sh` runs only `tests/*.rs`, as 5.0.1 follow-ups.

Found (2026-11-16): `lsp.rs:206-230` is the one reader of
`installed_plugins.json`; nothing reads `known_marketplaces.json` or runs the
`claude` CLI; no unit test writes to the real config directory, so the
leak-hunt extension is a guard, not the cause of the stray
`~/.claude/greenpiste` of 2026-11-15.

## Goal

After a migration (or an install) whose plugin Claude Code still registers
under the old id, or has not registered at all, greenpiste says so and names
the two commands that fix it, in `migrate`'s note, in `install --write`'s
output and as a failing `doctor` finding; and `scripts/leak-hunt.sh` runs every
test target of the workspace.

## Non-goals

- greenpiste running `claude plugin ...` itself, or writing either registry
  file: Claude Code owns them (a foreign tool's state), a hook must never spawn
  it, and the CLI is the supported way to change them.
- Finding what wrote the stray `~/.claude/greenpiste` of 2026-11-15.
- The old-name census (from 2026-12-13).

## Code review
- amont-agent: backend, lang:rust, unix, tui (architect left out: no new module or boundary, a read of a file greenpiste already reads)
- grh: backend (unix, lang:rust left out: one manifest pin and a changelog line)

## Behaviour

- **One registry reader.** `lsp.rs`'s `installed_plugins` stays the only
  reader of `installed_plugins.json`; a sibling `known_marketplaces(config) ->
  KnownMarketplaces` (same three-way shape: Missing, Unreadable(why), Document; `Unreadable` gains the io/parse reason in both enums so doctor can say it)
  reads `known_marketplaces.json`. Both move from `lsp.rs` to `settings.rs`,
  beside the other Claude-config readers, and `lsp.rs` calls them there.
- **What "stale" means**: one private classifier in `migrate_plugin.rs`
  judges a side (marketplace, plugin id, marketplace dir) against the registry
  files under `env.config_dir`; `Side` stays private. Two public entry points
  call it: the observed state (reads settings.json for the side it names;
  `doctor` and explicit `migrate` use it) and an intended state (the side a
  dry run or an install preview will write: the requested direction for
  `migrate --dry-run`, `plan.after` of the selected scope for `install`,
  `--project` and `--local` included). It returns:
  - `Current`: `known_marketplaces` has the side's marketplace with a
    `source.path` naming the side's marketplace dir (both canonicalised, raw text
    as fallback; a unit case reaches it through a symlink), and `installed_plugins`'s
    `.plugins` has a key equal to the side's plugin id;
  - `Stale { old_marketplace, old_plugin, missing_marketplace, missing_plugin }`:
    any of the other side's marketplace or plugin is still there, or the side's
    own is missing;
  - `Unknown`: either file is unreadable (said as such, never as current); a
    missing file is not `Unknown` but counts as the side's missing pieces;
  - `NotUsed`: settings.json does not register the plugin at all (nothing to
    say).
- **The fix, said the same way everywhere**: for each missing piece, in order,
  `claude plugin marketplace add <side marketplace dir>`, `claude plugin
  install <side plugin id>`; for each leftover of the other side, `claude
  plugin uninstall <other plugin id>`, `claude plugin marketplace remove
  <other marketplace>`. One function builds the list, shell-quoting any argument that needs it (a path with a space or a shell metacharacter), and a unit test covers a path with a space.
- **Where it is said**:
  - `migrate` (explicit runs only; a hook never prints): `run_migrate` emits
    registry diagnostics after successful settlement, including `AlreadyDone`,
    `Reversed` and `ReversedAlready`; diagnostics do not depend on executing
    the plugin step or retaining its note. `Stale` prints `Claude Code still
    lists ...; run:` followed by one command per line. `Unknown` prints the
    could-not-read explanation with no commands. `--dry-run` evaluates the
    intended destination and prints commands as `would: ` lines.
  - `install --write`: after `enabled clear-relay@greenpiste`, the same lines
    on stderr when the state is not `Current`; `install` without `--write`
    prints them as `would then need:` lines, also on stderr.
  - `doctor`, migration section: a failing finding `Claude Code's plugin
    registry still lists clear-relay@amont-agent` (or `has not registered
    clear-relay@greenpiste`), with the commands as its detail, one per line
    (`doctor::report` splits `detail` on newlines and sanitizes and indents
    each line, since `ui::sanitize` escapes `\n` as `\x0a`); `Unknown` is a
    failing `could not read Claude Code's plugin registry: <file>: <why>;
    check the file, or run claude plugin list`.
    `NotUsed` and `Current` add nothing. Failing although greenpiste cannot
    fix it (the plugin is off until it is fixed); the comment at
    `doctor.rs:379-381` is reworded to say so.
- **leak-hunt covers every test target**: besides each `tests/*.rs`,
  `scripts/leak-hunt.sh` runs `cargo test -p greenpiste-guard --lib`,
  `cargo test -p greenpiste-process --lib` and the binary's own unit tests
  (`cargo test -p greenpiste --bins`), each against its own canary (`--features test-hooks` only for the packages that declare it, `greenpiste` and `greenpiste-guard`), named
  `lib:<crate>` / `bin:greenpiste` in its output; `scripts/leak-hunt.sh lib:greenpiste-guard`
  runs one. A unit test without a scratch dir writes into the canary rather
  than migrating it, so every target is also `LEAK` when a file listing of the
  canary's `claude/` and `state/` (path, size, mtime) differs after the run,
  or anything appears under its `home/`. An unknown target name exits 2 with a
  usage line on stderr.

## Phases

- [x] Phase 1 — registry state and messages:
  `amont-agent/crates/greenpiste-guard/src/settings.rs`,
  `amont-agent/crates/greenpiste-guard/src/lsp.rs`,
  `amont-agent/crates/greenpiste-process/src/agents.rs` (its import of the moved reader, and the `InstalledPlugins::Unreadable` match at :200 now that the variant carries a reason),
  `amont-agent/crates/greenpiste-guard/src/migrate_plugin.rs`,
  `amont-agent/crates/greenpiste-guard/src/doctor.rs`,
  `amont-agent/src/main.rs`, `amont-agent/tests/migrate.rs`,
  `amont-agent/tests/doctor.rs`, `amont-agent/docs/migrate.md`,
  `amont-agent/CHANGELOG.md`.
- [x] Phase 2 — leak hunt covers unit tests: `amont-agent/scripts/leak-hunt.sh`.
- [ ] Phase 3 — release 5.0.1 and pin it: `amont-agent/Cargo.toml`,
  `amont-agent/greenharness.toml`, `grh/manifests/greenpiste.toml`,
  `grh/catalog.toml`, `grh/CHANGELOG.md`, `grh/Cargo.toml`; released,
  installed here.

## Verification

- Phase 1: unit tests of `registry_state` over scratch config dirs for each
  state (both files current; old plugin left; old marketplace left; new
  marketplace missing; new plugin missing; each file unreadable; settings not
  registering the plugin; a marketplace added through the old symlinked
  path), in both directions, with fixtures copying the shape of Claude Code's
  real `known_marketplaces.json` (`source.path`, `installLocation`); the
  command list for each, including a path with a space.
  `tests/migrate.rs`: on a scratch machine whose `plugins/*.json` hold the old
  ids, `migrate` exits 0 and its stderr names the four commands in order;
  `migrate --dry-run` says them as `would:` lines; after `--reverse` the note
  names the commands back; a hook run prints nothing extra; a `migrate` on an
  already-migrated machine and a repeated `--reverse` still print the
  diagnostics; an install preview, fresh and with `--project`/`--local`, names
  the same commands as the `--write` that follows; registry files never
  change. `tests/doctor.rs`:
  a stale registry is a failing (`✗`) finding naming the commands, each on its own indented line with no `\x0a`; a current
  one adds nothing; an unreadable file says `could not read`; `doctor` writes
  nothing (tree snapshot). `scripts/leak-hunt.sh` is clean on every target.
- Phase 2: `scripts/leak-hunt.sh` lists `lib:greenpiste-guard`,
  `lib:greenpiste-process` and `bin:greenpiste` with `ok`; a planted unit test
  that writes through `journal::record` without a scratch dir is reported
  `LEAK` (planted, run, removed).
- Phase 3: the release run concludes `success`; on this machine `greenpiste
  doctor` shows no registry finding (the registry was fixed by hand on
  2026-11-15, and `known_marketplaces.json` holds no old entry) and
  `greenpiste migrate --dry-run` says `nothing to do`; a scratch copy of the
  pre-fix registry shape gives the stale finding and the four commands.

## Rollback

- Phases 1–2 only add reads and messages: revert. Phase 3: reinstall 5.0.0.

## Outcome

Observed before the push (2026-11-16):

- Phase 1: `cargo test` green on the branch (a514bd75); `tests/migrate.rs` and
  `tests/doctor.rs` cover the stale, current, unreadable and preview cases.
- Phase 2, planted unit test calling `journal::record` with no scratch dir →
  expected `LEAK` → `LEAK lib:greenpiste-guard: the canary changed
  (> claude/amont-agent/journal.log …)`, twice; the plant removed (`cmp` with
  the clean copy, 0 `planted_leak` lines).
- Phase 2, two clean runs of every target → expected all `ok` → 43 `ok`, rc 0,
  both runs.
- `LEAK_HUNT_SNAPSHOT_FAIL=1 scripts/leak-hunt.sh migrate` → `SNAPSHOT-FAIL`,
  rc 1; `scripts/leak-hunt.sh lib:nope migrate` → usage on stderr, rc 2, nothing run.

## Decision log

- 2026-11-16 — Context was wrong: unit tests did write outside their scratch
  dirs. The attestation sweep's tests wrote the shared journal (now
  `journal::record_in` with the scratch state dir), and two `with_home` test
  helpers left `CLAUDE_CONFIG_DIR` set, so every later test wrote into their
  scratch dir; that hid the planted leak (`ok`) until a `RestoreConfigDir`
  guard put the variable back. Phase 2 therefore also touched
  `crates/greenpiste-guard/src/{journal.rs,assertions/mod.rs,assertions/bug_nudge.rs,session_state.rs}`
  and `crates/greenpiste-process/src/attestations.rs`. Unit-test targets run
  with `--test-threads=1`, so a test holding the variable cannot redirect a
  concurrent one.

- 2026-11-16 — say the commands rather than run them: the registry is Claude
  Code's, its CLI is the supported writer, and a hook may never spawn it.

<!-- panel: repos=amont-agent,grh adds= reviewers=backend,lang:rust,tui,unix body-sha=6c5b0946fbd4 -->
