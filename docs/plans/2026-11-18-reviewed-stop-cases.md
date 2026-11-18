---
status: active
branch: feat/stop-cases
repos: [amont-agent, decisions]
adrs: [ADR-0062]
programme: session-boundaries t20
plan-id: p-01M7P2DQ3WQJ20XB19VVE0HBKG
---
# Reviewed Stop cases, then clear-at-boundary to deny

## Review panel

👉 **Decide:** none — approve if replaying frozen Stops through the hook's own core is honest enough evidence to graduate to `deny`.
📍 amont-agent, decisions · 4 phases · 1 release · next: Phase 1. Panel: backend, lang:rust, tui, unix (all opus-5-5), codex.
**Changed by review:** `state.pending` frozen; the hook's real byte window rebuilt, and redaction proven replay-equivalent (codex); labels read the original transcript.
📄 Full reviews: [2026-11-18-reviewed-stop-cases.reviews.md](2026-11-18-reviewed-stop-cases.reviews.md)
**Verdicts:** 3 approve, 1 approve-with-changes after the codex delta; 13 carried; words 1874 → 2570 (×1.37, the codex highs exempt); remove 0, merge 0.

## Picture

```
 today
 -----
 journal: "advised"  --x-->  explain --format cases   (exports nothing)
 corpus:  4 nomatch command lines  -->  graduate: 4 of 12, refused

 after this plan
 ---------------
 journal line --explain--> Stop fixture --label--> corpus line
   (session, time)          (frozen payload,        match / nomatch
                             trimmed transcript,         |
                             frozen git facts)           v
                                     corpus check --> decide() --> fires?
                                                     (the hook's own core)
                                                         |
                                                         v
                                     graduate clear-at-boundary --to deny
```

An advised Stop used to leave only a journal line. After this plan it
becomes a labelled fixture that the corpus replays through the hook's own
core, and the graduate gate counts it.

## Context

t10 decided `clear-at-boundary` goes to `deny`. `greenbelay graduate`
refused: 4 reviewed cases, and it needs 12, of which 4 must be negatives.
The corpus (`crates/greenbelay-guard/tests/corpus/*.cases`) holds command
strings, plus one non-command form, `ReadCase`, which `read_judge` judges
per rule (`corpus.rs`). The rule decides at Stop, from the payload, the
transcript tail, the state file, a phase fact and a git check, all inside
`clear_boundary::judge_boundary` (`greenbelay-process`). Nothing replays
that judgement, and `explain --format cases` scans Bash calls only.

**Labels judge what the Stop saw** (the person, 2026-11-18). `match` means
the transcript up to that Stop shows a real boundary with nothing in
flight, even if work went on afterwards. The early-fire share (an advice
followed by an editing turn) is counted separately and recorded in t20. It
does not gate.

## Goal

- A corpus case form for a Stop. Its line is `<verdict>\tStop <name>`, and
  it names a fixture that `corpus check` and `graduate` replay through the
  hook's own code. The installed binary carries the fixtures compiled in.
- `explain clear-at-boundary --format cases` exports the journal's live
  advices and in-flight silences as fixtures with `?` lines.
- At least 8 live boundaries labelled, including advices that came before
  an editing turn. Then `greenbelay graduate clear-at-boundary --to deny`
  passes without `--force`, on the released binary.

## Non-goals

- `plan-phases-open` cases. The form is generic (a judge per rule id), but
  only clear-at-boundary gets a judge here. Its cases are a follow-up
  ticket.
- Replaying the git checks. `check_repositories` and the phase fact touch
  repositories that have moved since, so the fixture freezes their
  results. The corpus therefore measures the transcript and payload half of
  the judgement, and the plan says so in the corpus header.
- Changing what the rule fires on (a disagreement goes to the person; see
  the Decision log).

## Code review

- amont-agent: backend, lang:rust, unix, tui
- decisions: none — markdown only

## Behaviour

### 1. One pure core (`greenbelay-process/src/clear_boundary.rs`)

`judge_boundary` is split. Its new core, `decide`, keeps every branch of
today's judgement. It takes:

- a `StopView`, with `cwd`, `permission_mode`, `background_busy`,
  `stop_hook_active`, and the three facts `payload_says_silence` reads from
  `last_message` (it ends with `?`, it has a `WAITING:` line, it contains
  `MARKER`);
- the tail entries;
- the prior `State`;
- the stance;
- three closures, `fact()` for the `PhaseFact`, `repos(dirs)` for the
  `Repositories` result, and `persist(&State) -> bool` for the state write
  (the live one writes the file; the replay one returns `true`). A failed
  write keeps today's order (`state-unwritable` first) and today's
  suppression of the advice; a unit test with a failing `persist` pins both.

It returns the `Decision`, the `State` to persist (or none), and the
journal records as `(outcome, what)` pairs. `on_stop` and `judge_boundary`
become the wrapper that does the I/O: they read and write the state file,
journal the records, and pass the live git closures. The hook's behaviour
stays the same, and its journal lines change only by the added
` uuid=<id>`. The existing `cargo test --test clear_at_boundary` suite
proves this, and its assertions are not edited in Phase 1.

`judge_case(json) -> Result<bool, String>` parses a fixture, builds the
inputs, and runs `decide` at `deny` with closures that return the frozen
facts. It fires when the result is a `Continue`. The replay runs at `deny`
because `deny` is the stance being graduated to. A malformed fixture is an
`Err`, which is a disagreement, as a malformed `ReadCase` is today.

The uuid of the tail's last entry is added to the journal excerpt as
` uuid=<id>`, so that later exports find their Stop exactly. That entry is
the Stop's own position and is unique per Stop, whereas
`window.prompt_uuid` is shared by every Stop of a turn.

### 2. The case form (`greenbelay-guard/src/corpus.rs`)

- `StopCase::parse` reads `Stop <name>`, where the name has no spaces and
  no path separators. It mirrors `ReadCase::parse`: `None` when the line
  is not a Stop case, and `Some(Err)` when it is malformed.
- Fixtures live at `tests/corpus/stop/<rule>/<name>.json`. They are
  compiled in through an `EMBEDDED_STOP` table, kept by hand like
  `EMBEDDED`, and a test fails when a file on disk has no entry or an
  entry has no file. This avoids repeating the `path_for` bug, where the
  installed binary saw 0 cases.
- Guard cannot call process (process depends on guard). So
  `score_cases_with(rule, cases, stop: Option<StopJudge>)` takes the judge
  as `fn(&str rule_id, &str fixture) -> Option<Result<bool, String>>`.
  `score` and `score_cases` keep their signatures, pass `None`, and a Stop
  case judged with `None` is a disagreement. A run that cannot judge
  therefore never passes quietly.
- `src/main.rs` passes `greenbelay_process::clear_boundary::stop_judge` to
  `corpus check` and to `graduate` (`graduate::assess_score`, which
  already takes a score).
- The corpus test that replays every embedded file moves its Stop half
  into a root-crate test, `tests/corpus_stop.rs`, which links both crates.
  The guard crate's own test asserts that every Stop line it meets is
  reported as needing the judge, not skipped.

Fixture shape (JSON, reviewed by eye):

```
{ "source":  { "session": "7458c00c", "journal": 1794960484,
               "outcome": "advised", "what": "worktree remove" },
  "payload": { "cwd": "~/Developer/Perso/decisions",
               "permission_mode": "bypassPermissions",
               "background_busy": false, "stop_hook_active": false,
               "ends_with_question": false, "waiting_line": false,
               "marker": false },
  "state":   { "key": "", "advices": 0, "pending": null },
  "facts":   { "phase": "none", "repositories": "clean" },
  "early":   true,
  "entries": [ ...trimmed main-chain and sidechain entries... ] }
```

`early` is set by the exporter when the next person turn ran an Edit,
Write, NotebookEdit, `git worktree add` or `mcp__relais__run`, the same
signal as `work_in`. It is read by the share count and never by the judge.

### 3. The exporter (`src/main.rs` explain arm, `clear_boundary::export`)

`greenbelay explain clear-at-boundary --format cases [--sample N] [--out DIR]`
(`--out` defaults to the build checkout's fixture directory; the directory
written is printed on stderr):

1. It reads the rule's journal lines (`journal.log*`), keeping `advised`,
   `denied`, and the in-flight silences `pending-work`, `pending relais`
   and `pending background`, plus the `pending <reason>` holds from a
   not-clean repository. Each of these is a Stop that reached a boundary,
   so they are the real negatives. `no boundary`, the 302 of
   them, is left out because it measures nothing about a boundary.
2. It finds the transcript by the session prefix under the transcript
   roots. A Stop whose transcript is missing or whose prefix matches more
   than one file is skipped with one stderr line, and stderr ends with a
   count. stdout carries only cases lines. Exit 0 when at least one fixture
   is exported, 1 when none is, 2 for usage. Without a `uuid=` it cuts the entries at the last main-chain
   entry timed at or before the journal second.
3. It freezes the facts the outcome implies. For `advised` and `denied`:
   repositories `clean`, phase not `Working`, the advice count equal to
   that session's earlier advices, and a key that differs. For
   `pending-work`, and for any Resumed boundary, `state.pending` (uuid,
   command, what, dirs) is rebuilt and frozen. It comes from the session's
   earlier `pending <reason>` line and from `boundaries()` over the
   transcript up to that Stop. For `pending relais`, the relais state is
   read from the transcript and is not frozen. For `pending background`,
   the flag can only be inferred from the outcome, so the fixture marks it
   `"inferred": true`. A hold from a not-clean repository freezes
   `"repositories": {"not_clean": "<reason>"}`.
   Before trimming, reconstruct the hook's actual input window from the
   original JSONL bytes ending at the selected Stop position. Apply the
   same chunk growth, partial-line handling, cap and stopping predicate as
   `on_stop`, including the reconstructed prior pending UUID. Record the
   selected byte bounds and capped flag; do not feed the entire historical
   prefix to `decide`. Cases whose window cannot be established remain
   exploratory and do not count toward graduation. Test a relais start
   outside the live window, a pending UUID that requires expansion, and a
   capped tail; each must reproduce the live window and decision.
4. It trims and redacts. Kept: entry `type`, `uuid`, `isSidechain`,
   `isMeta`, `message.usage`, content-block types, tool names and IDs,
   tool-result `tool_use_id` references, `mcp__relais__decide.input.run`,
   Bash commands through the backtest's existing redaction, the relais
   phrases `relais_messages` reads, and the prompt and injected markers
   `is_person_prompt` needs (not the prompt text). Ordinary tool results
   retain structural stubs so their entries cannot become prompts.
   Dropped: assistant prose and ordinary result bodies, plus `last_message`
   (only its three flags are kept). Every path under `$HOME` becomes `~`.
   Before export, compare the original-window and redacted-window replay
   under identical inputs: decisions, state transitions and journal reasons
   must agree after path normalization. A mismatch excludes the fixture
   from graduation. Tests cover ordinary tool results, metadata prompts,
   paired relais starts/results and a completed relais decision.
5. It replays each fixture through `judge_case`. A fixture whose replay
   disagrees with its journal outcome is written with a `# replay
   differs: …` comment above its `?` line, so it is never labelled blind.
6. It builds every fixture in memory first. It then writes each one under
   the `--out` directory as a temp file in the same directory, followed by
   a rename, so that SIGPIPE (`| head`) or SIGINT never leaves a truncated
   fixture. The `?` lines are printed last, after every write.
   - A fixture is named `<session8>-<journal second>`. An existing file is
     never overwritten: one whose content differs is named on stderr and
     skipped.
   - A `Stop <name>` already in the `.cases` file is not printed again, so
     a second run appends nothing.
   - Stdout carries only case lines, so `>> clear-at-boundary.cases` stays
     clean. Stderr ends with one summary: `wrote N fixtures to <dir> (K
     replay differs, S already exported, M skipped)`.
   - With no `--out` and no `checkout_present()`, it writes nothing to
     stdout and exits 2, naming the command to run from the checkout.

### 4. Labelling and graduation

The labeller reads the original local transcript through the identified
Stop, including the person's request, assistant commitments and relevant
command results, before assigning a label. The redacted fixture is the
replay artifact, not the sole evidence for the label. Each reviewed case
records a sanitized rationale and a stable source locator (session and
Stop position); raw prose remains local. A case without enough source
evidence stays `?` and does not count toward graduation. Labels remain
about what was visible at that Stop, without later-turn hindsight:

- `match`: a boundary with nothing in flight;
- `nomatch`: a pending or in-flight Stop, or an advice whose boundary was
  misread.

The labelled set is at least 12 live boundaries (the 4 command negatives are vacuous and are not counted toward the bar): at least 6 advices, at
least 2 of them `early`, and at least 6 in-flight silences from `pending-work` or `pending relais` (`pending background` is inferred and does not count). With the 4
command negatives, the gate sees at least 16 reviewed cases. The
early-fire share over every exported advice (labelled or not) is recorded
in t20's Resolution.

## Phases

- [x] Phase 1: `decide` core, the wrappers, the journal `uuid=`, and
  `judge_case`, in `crates/greenbelay-process/src/clear_boundary.rs`. The
  Stop suite `tests/clear_at_boundary.rs` passes unchanged. New unit tests
  replay hand-built fixtures for an advice, a `pending-work`, a `pending
  relais`, a `background_busy`, and a malformed fixture.
- [x] Phase 2: `StopCase`, `EMBEDDED_STOP` and `score_cases_with` in
  `crates/greenbelay-guard/src/corpus.rs`, and the wiring for `corpus
  check` and `graduate` in `src/main.rs`. Also the root-crate corpus test,
  `tests/corpus_stop.rs`, and a test that fails when any embedded Stop
  fixture exceeds 64 KiB.
- [ ] Phase 3: the exporter (`crates/greenbelay-process/src/clear_boundary.rs`
  and the explain arm in `src/main.rs`). Freeze the 14 sessions'
  transcripts first, because transcripts get deleted: run the export once
  with `--out` set to a scratch directory outside the embedded tree
  (`~/.local/state/t20-stop-export`). Only the fixtures picked for
  labelling move into
  `crates/greenbelay-guard/tests/corpus/stop/clear-at-boundary/` and the
  `EMBEDDED_STOP` table, with the total size stated in the PR.
- [ ] Phase 4: labelling (by the agent, per the person's label rule) in
  `crates/greenbelay-guard/tests/corpus/clear-at-boundary.cases`, with
  `corpus check` green. Then greenbelay's release with the tag-release
  skill (`CHANGELOG.md`), brew upgrade, and `greenbelay graduate
  clear-at-boundary --to deny` with no `--force`. The early-fire share and
  the result go into
  `docs/programmes/session-boundaries/t20-transcript-rule-corpus.md` and
  `map.md` in decisions.

## Decision log

- 2026-11-18: labels judge what the Stop saw, not hindsight (the person).
  The early-fire share is reported and does not gate.
- 2026-11-18: the git facts are frozen, not replayed. The world they read
  has moved, and replaying `check_repositories` against today's
  repositories would measure today, not the Stop.
- 2026-11-18: the judge is injected (`fn` pointer), not moved into guard:
  the core needs process's transcript helpers.
- If a labelled case disagrees with the rule at Phase 4, the graduation
  stops and the disagreement goes to the person, with options (narrow the
  rule in a new plan, relabel, or stay at `advise`). `--force` is never
  used.

## Verification

- `cargo test --workspace` passes, and `cargo test --test
  clear_at_boundary` passes with no edits to its assertions (Phase 1
  behaviour unchanged). A journal-line diff test shows that only
  ` uuid=<id>` was added. A test with a long `what` plus ` tail-capped`
  shows that the uuid survives `redact_and_trim`, which caps at 200
  characters.
- Export over the 14 sessions: prints fixtures per outcome and the `replay differs` count; expected 0 for `advised`, each other one listed in t20.
- A hand-made fixture labelled the wrong way turns `corpus check` red, and
  the failure names the line, the fixture path and why. `Disagreement`
  gains a `why`, and `corpus check` prints `<rule>.cases:N expected <v> —
  Stop <name> (stop/<rule>/<name>.json): <why>`. `<why>` is one of
  `fixture malformed: <err>`, `no Stop judge in this build`, `rule fired at
  deny` or `rule did not fire at deny`. A test pins each one.
- Running the export twice leaves the `.cases` line count and the
  fixtures unchanged. `explain … --format cases | head -1` leaves no
  truncated or temp fixture behind.
- Guard alone (`cargo test -p greenbelay-guard`) reports a Stop case as
  needing its judge and never counts it as agreeing.
- An installed-like run: copy the release binary outside the checkout (no
  `CARGO_MANIFEST_DIR` tree) and run `HOME=<scratch dir>
  GIT_CONFIG_GLOBAL=<scratch dir>/.gitconfig XDG_CONFIG_HOME=<scratch
  dir>/.config <copy> graduate clear-at-boundary --to deny`. `graduate` has no dry run, and it writes
  the stance with `git config --global`, so the scratch `HOME` takes that
  write. Its report counts the embedded Stop cases. The live `~/.gitconfig`
  stance is checked unchanged afterwards.
- Redaction: `grep -rn "/Users/"` over the committed fixtures finds
  nothing, the secret-shape redaction test runs on every fixture's
  commands, and a read by eye finds no assistant prose.
- Phase 4: on the brew-installed release, `greenbelay graduate
  clear-at-boundary --to deny` prints every line ✓, and `greenbelay status`
  shows the rule at `deny`. The next live boundary past 200k journals
  `denied`, and the session writes its handoff.

## Outcome

- Within 7 days of the graduation, journal lines show `clear-at-boundary
  deny denied`, and t12's handoff count (due 2026-11-24) includes the
  denied boundaries. Rollback: `greenbelay demote clear-at-boundary` if
  denied boundaries are followed by editing above the measured early-fire
  share.

<!-- panel: repos=amont-agent,decisions adds= reviewers=backend,lang:rust,tui,unix,codex body-sha=f5e6add40667 -->
