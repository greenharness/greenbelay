---
status: active
branch: fix/clear-boundary-cd-and-vars
repos: [amont-agent, decisions]
adrs: [ADR-0062, ADR-0077]
programme: session-boundaries t23
plan-id: p-01M7RG48JK13S5NB12126MYKRH
---
# clear-at-boundary follows a `cd` and a variable set in the same command

## Review panel

👉 **Decide:** none — approve if following a chained `cd` and literal `NAME=` assignments is worth one new journal row.
📍 amont-agent, decisions · Phase 1 done, backtest 394/404 (97.5%) · next: merge, then Phase 2 on or after 2026-11-25. Panel: backend, lang:rust, tui, unix (opus), codex.
**Changed by review:** the walk starts at the call's own cwd; conditional and compound assignments go Unknown; redirects, unassigned names and `dir=` holds handled.
**Verdicts:** 4 approve-with-changes; codex rework (3 high applied), then backend approve-with-changes; 6 carried; words 1428 → 2369 (×1.66, high findings exempt); remove 0, merge 0. Delta 1 (first backtest 93.9%, below the bar): backend approve-with-changes twice, 4 applied, 3 carried.
📄 Full reviews: [2026-11-19-clear-boundary-follows-cd.reviews.md](2026-11-19-clear-boundary-follows-cd.reviews.md)

## Picture

```
 cwd       ~/Developer/Perso/amont-agent-wt-rel-641    (where the call began)
 command   LIVE=~/Developer/Perso/decisions; …;
           git -C "$LIVE" worktree remove ../decisions-wt-t21

 today   -C "$LIVE" joined to the cwd
           = …/amont-agent-wt-rel-641/$LIVE -> not a dir -> skipped
           -> only the cwd is checked -> a false "clean"
 after   walk the command's simple commands in order:
           LIVE=~/…/decisions     vars[LIVE] = ~/…/decisions
           git -C "$LIVE" …       dir = ~/…/decisions -> checked
         cd ~/repo && git worktree remove ../x
           here = ~/repo          no -C -> dir = here -> checked
         (cd …), cd -, NAME=$(…), for-loop vars
           unknown -> today's dirs + journal "unresolved …"
```

The rule already checks whatever directory it is given. Walking the
command in order hands it the directory the command acted on, and a shape
it cannot read is journalled instead of guessed.

## Context

t23 counted the boundary calls in the main-chain transcripts (decisions
PR #258, `docs/programmes/session-boundaries/boundary-shapes.py`): 1,545
calls from 2026-10-15 to 2026-11-18. Of those, 551 (36%) check the wrong
repository, or none besides the cwd. `boundaries()`
(`crates/greenbelay-process/src/clear_boundary.rs:286`) classifies each
simple command alone. `chain()` (`:530`) resolves `-C` words against the
payload cwd, so it misses two things:

- a `cd` earlier in the same command. 112 calls where the shell reset
  outside the working dirs, so the payload cwd is not the `cd` target;
- a `NAME=value` assignment earlier in the same command (309 calls).
  `-C "$NAME"` is joined to the cwd as the literal `$NAME`, and the result
  fails `is_dir()` in `check_repositories` (`:794`).

Either way the miss is a false "clean", never a false hold: the cwd is
always checked as well. t23's Resolution decides to follow only the shapes
that need no guessing (421 of the 551), and to journal the rest as
`unresolved`.

## Goal

For each boundary, the rule checks the repository the command acted on
whenever the command itself fixes it: a chained top-level
`cd <literal|home|$NAME>`, or a standalone `NAME=<literal|home>`. A shape
it cannot read changes nothing in the decision; it adds one `watched
unresolved <shape> <what>` journal row. `<shape>` is one of `subshell`,
`command-subst`, `cd-dash`, `cd-piped`, `cd-or`, `keyword`, `loop-var`,
`export`, `unassigned`, `not-literal`, `conditional`.

## Non-goals

- Subshells `( … )`, `$(…)` and backtick values, `for`/`read`/`export`
  assignments, `cd -`, `$PWD`, `pushd`: unresolved, never guessed.
- A boundary command whose first word is a keyword (`do git …`,
  `then git …`). Today these are not seen as boundaries at all. That gap
  goes to the inbox as a follow-up.
- Positional worktree paths (`worktree remove ../x`): only `-C` and the
  current directory choose the repository, as today.
- No change to the advice or deny text, to the stance, or to t22's window.
  The release waits until 2026-11-25.

## Phases

### Phase 1: walk the command in order (done 2026-11-19)

In `crates/greenbelay-process/src/clear_boundary.rs`:

- Add a separator to the output of `shell_words` (`:353`), as a new
  function. `segments` keeps its signature for `adds_worktree` (`:1193`).
  The new function returns each simple command with the operator that
  ended it: `;`, newline, `&&`, `||`, `|` (and zsh `|&`), `&` or the end.
  A `&` after `<`/`>` or before `>` (`2>&1`, `>&2`, `&>`, `&>>`) belongs
  to a redirect, not an operator; redirect words (`N>`, `>file`, `N>&M`)
  are dropped before a `cd`'s words are counted.
- Add `struct Walk { here: Option<PathBuf>, vars: BTreeMap<String, Value> }`,
  where `Value` is `Known(String)` or `Unknown`. `boundaries()` creates one
  `Walk` per Bash call. It starts at the `cwd` of the transcript entry
  that carries the call, which is where the call began; the Stop payload's
  cwd can already be past the call's own `cd`, and a relative `cd ./child`
  would then be applied twice. Without an entry `cwd` (an exported case
  today), it starts at the payload cwd. The case export's `trim`
  (`clear_boundary_export.rs:951`) keeps the entry `cwd`, home written as
  `~`, so a case replays the same. A test: start `/w`, payload `/w/child`,
  `cd ./child && git worktree remove ../x` gives `/w/child`. The walk steps
  through the commands in order:
  - **Assignment.** A command made only of `NAME=value` words sets
    `vars[NAME]`. The value is `Known` when it holds no unquoted or
    double-quoted `$` or backtick, except a leading home (`home_suffix`),
    which is expanded. Anything else is `Unknown`. `export`, `local`,
    `readonly`, `declare`, `read NAME` and `for NAME in` set
    `vars[NAME] = Unknown`. Between a compound command's opening keyword
    (`for`, `while`, `until`, `if`, `case`, `{`) and its closing one
    (`done`, `fi`, `esac`, `}`), any name assigned or bound inside becomes
    `Unknown`, and a `cd` inside sets `here = None`. A binding the compound
    does not touch keeps its value, so the 4526105a command, whose
    `if … fi` never assigns `LIVE`, still resolves.
  - **cd.** `cd` with no word, or `cd ~`, goes to home. `cd <word>` takes
    the word's resolved directory (below) against `here`. `cd -`, a `cd`
    with two words, an `Unknown` word, or a `cd` ended by `|` or a lone
    `&` (it runs in a subshell), ended by `||` (what follows runs only
    when it failed) or after `||` (it may not have run), or
    last in a pipeline (zsh runs it in the current shell) sets
    `here = None`. An assignment ended by `|` or `&`, or after a `|`,
    sets `Unknown`. An assignment or `cd` that follows a `&&` or `||` is
    conditional: it holds for the rest of that `&&`/`||` list, and when the
    list ends (`;`, newline, `&`) a conditional `cd` sets `here = None`.
    A name the list set becomes `Unknown` (shape `conditional`) when it
    had an earlier binding in the command (looked up before the assignment writes `vars`), so `R=/a; false && R=/b; git -C
    "$R" …` is unresolved, never `/b`. A name with no earlier binding
    set after a `&&` keeps its value (after a `||` it is `Unknown`,
    `conditional`: it ran only when the left side failed): had the assignment not run, the name holds what the
    environment gave it, which the walk reads as `unassigned` and checks
    as today; so following it only adds a check, never drops one. An added check can add a hold: when the list's head failed (`gh pr merge` refused), the later `git -C "$LIVE"` ran with an empty `-C`, and the follow checks a repository the command never touched. So
    `gh pr merge 1 && LIVE=~/repo && git -C "$LIVE" fetch; git -C "$LIVE"
    worktree remove x` checks `~/repo` (19 of the backtest's 24 misses).
    A first word that
    starts with `(` sets `here = None` for the rest of the command.
  - **A word's directory.** A word starts with home, or with `$NAME`,
    `${NAME}` or `"$NAME"`, optionally followed by `/rest`. The variable
    is bare or double-quoted, as `home_suffix` reads quoting. A `Known`
    variable is replaced by its value; a variable never assigned in the
    command is `Unknown`: the shell still inherits the environment
    (`$PWD`, `$OLDPWD`, `$TMPDIR`, the shell snapshot's exports), so the
    two reviewed `"$LIVE"` fixtures keep today's dirs and journal
    `unresolved`. Any other `$` in the word is
    `Unknown`, as is a bare `~` that is not home (`~-`, `~+`, `~user`),
    a leading `=`, an unquoted `*`, `?`, `[` or `{`, a `cd` word `-N` or
    `+N` (zsh directory stack), and a relative `cd` word when `CDPATH`
    is set. The result is folded with `resolve`/`fold` against `here`.
- `classify` takes the `Walk`. `chain` starts from `here` instead of
  `cwd`. With no `-C`, `dirs = [here]` unless `here` is both where the
  call began and the payload cwd. So a call with no `cd` that began
  outside the Stop's cwd now checks where it began (9 of the backtest's
  796 already-correct calls gain that check, each the repository the
  call ran in). When `here` or
  a `-C` word is unknown, `dirs` is what it is today, and the `Boundary`
  carries `unresolved: true`. `pending_from` sets it when any boundary found is unresolved, as it already merges their dirs.
- `judge` keeps `Repositories::NotClean.directory` (`:1402`, dropped
  today). When that directory is not the cwd's top, the hold row is
  `pending <reason> dir=<its basename>`, so a row whose `repo` is the cwd
  still names the repository that is not clean. `Kind::of` and
  `matches_reason` (`clear_boundary_export.rs:132`, `:168`) drop a
  trailing ` dir=…` before parsing and comparing, so the exported reason
  (`:678`) has no suffix and older `pending <reason>` lines still match a
  replay. A round-trip test (journal row → exported case → replay →
  match) pins both.
- `Boundary` and `Pending` gain `unresolved: bool`. Neither derives serde:
  the state write (`json!`, `:986`) emits it, and `parse_pending` (`:933`)
  reads it with `as_bool().unwrap_or(false)`. `PendingFixture` gets
  `#[serde(default)]`. A unit test loads a state file without the key.
- `Ledger::judge`: for an own boundary with `unresolved`, record
  `("watched", "unresolved <shape> <what>")` once, before the other checks. The
  decision does not change. `Kind::of` in `clear_boundary_export.rs`
  maps this body to `None`, so the case export ignores it. A unit test
  pins that. A test in `greenbelay-guard/src/journal.rs` pins how
  `tally` counts a `watched unresolved` row: it matches the outcome
  field only, so the row counts as `watched`, and `watched` means rows,
  not Stops (one unresolved Stop that is then held counts 2).

Tests, in the `tests` module of `clear_boundary.rs`, written red first
against `origin/main`:

| Command (cwd `/w`, home `/h`) | dirs | unresolved |
|---|---|---|
| `cd /h/repo && git worktree remove ../x` | `[/h/repo]` | no |
| `cd ~/repo; git commit -m "docs(plan): p"` | `[/h/repo]` | no |
| `LIVE=~/repo; git -C "$LIVE" worktree remove x` | `[/h/repo]` | no |
| the 4526105a command (`LIVE=~/Developer/Perso/decisions; … git -C "$LIVE" worktree remove ../decisions-wt-t21`) | `[/h/Developer/Perso/decisions]` | no |
| `W=/h/a; cd "$W" && git worktree remove ../b` | `[/h/a]` | no |
| `R=/h; git -C "$R/relais" worktree remove x` | `[/h/relais]` | no |
| `git -C "$LIVE" worktree remove x` (never assigned) | today's | yes |
| `cd /h/a && git -C ~/b worktree remove x` | `[/h/b]` | no |
| `(cd /h/a && git worktree remove ../b)` | today's | yes |
| `LIVE=$(git rev-parse --show-toplevel); git -C "$LIVE" worktree remove x` | today's | yes |
| `for r in a b; do LIVE=/h/$r; git -C "$LIVE" worktree remove x; done` | today's | yes |
| `cd - && git worktree remove x` | today's | yes |
| `cd /h/a \| cat; git worktree remove x` | today's | yes |
| `cd /h/a 2>&1 && git worktree remove ../b` | `[/h/a]` | no |
| `cd /h/a >/dev/null && git worktree remove ../b` | `[/h/a]` | no |
| `cd /h/a \|\| git worktree remove x` | today's | yes |
| `cd ~- && git worktree remove x` | today's | yes |
| `LIVE='$x'; git -C "$LIVE" worktree remove y` (single-quoted, literal) | `[/w/$x]` | no |
| `cd repo && git worktree remove ../x` (relative, `CDPATH` unseen) | today's | yes |
| `if c; then cd /h/a; fi; git worktree remove x` | today's | yes |
| `R=/a; false && R=/b; git -C "$R" worktree remove x` | today's | yes (`conditional`) |
| `gh x && LIVE=~/repo && git -C "$LIVE" fetch; git -C "$LIVE" worktree remove x` | `[/h/repo]` | no |
| `test -d /h/a \|\| LIVE=/h/b; git -C "$LIVE" worktree remove x` | today's | yes (`conditional`) |
| `cd /h/a && git worktree remove ../x`, the call began in `/h/a` | `[/h/a]` | no |

- A unit test on `decide` with an unresolved boundary asserts one
  `watched unresolved subshell worktree remove` record, and the same decision as
  without the flag.
- The corpus is unchanged: every Stop case keeps its outcome
  (`greenbelay corpus check`). Clean fixtures stay clean for any new
  directory. In a `per_directory` fixture, a directory the map does not
  name is clean (`RepositoriesFixture::PerDirectory`).

### Phase 2: release after t22's window, and close t23

- Merge on green now. The installed binary does not change until the
  release.
- On or after 2026-11-25, once t22 is recorded, release greenbelay
  (minor: a new journal row) with the `tag-release` skill
  (`Cargo.toml`, `CHANGELOG.md`) and install it.
- decisions, one PR
  (`docs/programmes/session-boundaries/t23-cd-and-same-command-variables.md`,
  `map.md`): t23's Resolution gets the release, the inbox
  follow-up for keyword-prefixed boundaries becomes a ticket, and the map
  Handoff is updated.

## Code review

- amont-agent: backend, lang:rust, unix, tui (the new `unresolved` and `dir=` journal rows are read in `explain`)
- decisions: none — markdown only

## Verification

- The Phase 1 table, red on `origin/main` for the rows marked "no" whose
  dirs differ from the cwd, and green after.
- Backtest (ADR-0060): run the branch `boundaries()` over the commands
  of decisions#258 (to 2026-11-18) before merging. Of the 422 followable
  calls the rerun counts (421 in t23), 29 are no boundary to the rule (closing plan commits,
  keyword-prefixed calls), so the bar is a share of the 393 it sees.
  Expected: at least 95% of them resolved to the directory the command
  acted on, and each of the 796 calls today already checks correctly
  keeps that directory; each disagreement is listed in t23 with the
  counts by shape. Below that bar, hold the merge and return the plan to
  review. First run (delta 1): 369/393 (93.9%), 796/796 kept; 19 of the
  24 misses were the `&& NAME=` shape this delta follows. Rerun after
  Delta 1, before merging: expected 388/393 and 796/796 kept; the 9
  calls that gain a check and the 29 excluded rows listed by reason in t23.
- `cargo test -p greenbelay-process` and `greenbelay corpus check` with
  the branch binary: every Stop case agrees, as before.
- Piloted, on the branch binary in a scratch `HOME`/state dir: replay the
  real 4526105a Stop (exported with `greenbelay explain clear-at-boundary
  --format cases`). Expected: dirs `~/Developer/Perso/decisions`, no
  `$LIVE` in any saved `dirs`.
- Piloted live, on the branch binary with `--hooks-dir` only (never
  `install` over the machine binary): a Stop whose boundary is
  `cd <probe> && git worktree remove ../x` with a modified file in the
  probe repo, cwd elsewhere. Expected: `pending uncommitted changes`, no
  Continue. Commit the file and repeat. Expected: Continue.
- In the scratch state, after one `(cd … && git worktree remove …)` Stop,
  `greenbelay explain clear-at-boundary` shows one `unresolved subshell`
  row, and the hold row names `dir=<probe>`.
- After the release (2026-11-25 or later): rerun `boundary-shapes.py`
  over the following 7 days' transcripts. Count the `unresolved` rows, by
  shape, against the Stops with an own boundary in the same window (a row
  is written only for a Stop's last own boundary). Expected: at most 15%
  of own boundaries are unresolved (t23: 130/1,545 calls, 8%).
  Recorded in t23.
- In the same 7 days, list every hold whose `dir=` is not the cwd's top, with the call's tool result. Expected: no hold from an `&& NAME=` follow whose list head failed; one returns unbound follows to `Unknown` (`conditional`).

## Verification record (Phase 1, 2026-11-19)

| Check | Input | Expected | Observed |
|---|---|---|---|
| Phase 1 table | `cargo test -p greenbelay-process` | red on the stub, green after | red 4/4 at 5469f4e0 (`#[ignore]`d to commit), green at 899addac with every row unchanged |
| Backtest (ADR-0060) | boundary-shapes.py rows to 2026-11-18, branch `boundaries()` with entry cwd and the cwd after the call | ≥95% of the followable boundaries the rule sees, today's dirs kept | before Delta 1: 369/393 (93.9%), held; after Delta 1: 385/393; final tree: 394/404 (97.5%), 846/846 kept. The 8–10 misses: exporter ground-truth errors where the branch checks the right repository (`cd ~/Perso && L=amont-agent`, `LIVE=../decisions`, a commit inside a heredoc written to a script) and `cd-or` rows, journalled |
| Corpus | `greenbelay corpus check`, branch binary | every Stop case agrees | `clear-at-boundary 20 reviewed (11 negative), precision 100%`, on 01964700 and on 899addac |
| Replay 4526105a | its exported Stop, branch `boundaries()`, home `~` | `~/Developer/Perso/decisions`, no `$LIVE` | `dirs=["~/Developer/Perso/decisions"]`, resolved |
| Pilot, dirty | scratch HOME at deny, `cd <probe> && git worktree remove ../x`, cwd elsewhere, probe modified | held, no Continue | no output; journal `pending uncommitted changes dir=probe`; pending dirs `[…/probe]` |
| Pilot, clean | same, probe committed and pushed | Continue | `{"decision":"block",…}`, journal `denied worktree remove` |
| Pilot, subshell | `(cd <probe> && git worktree remove ../z)` | one `unresolved subshell` row | `watched … unresolved subshell worktree remove`, then the decision as without it |
| Control | installed 6.5.0, dirty pilot | the bug: a Continue | `{"decision":"block",…}` |

The fix to `here` (08ba504b) and the review fixes also widen what is
checked: 9 already-correct calls gain their entry cwd (a call with no `cd`
that began outside the Stop's cwd), and reading a heredoc body raw finds
67 real `docs(plan)` commits after an apostrophe-bearing heredoc that the
quote parser used to swallow.

## Implementation review

- implementation-review (opus): round 1 approve-with-changes, 3 flag arguments, a weak name, a test (fixed); delta approve; 49k + 50k, 26 s + 40 s.
- backend (opus): round 1 4 fixed (idempotent row, own boundary, absolute entry cwd, `dir=` with a space); delta 2 lows to the inbox (row at the cap, `command -v`); 77k + 46k.
- lang:rust (opus): round 1 5 fixed (`Shape` enum, dead branch, `NotCleanOf`, idioms); delta 5 lows to the inbox; 72k + 46k.
- unix (opus): round 1 4 fixed (comments, pushd/builtin/eval, raw heredoc body, `|| exit`); delta `|| return` deliberate: zsh exits on a top-level `return` (checked), the Bash tool runs zsh; 72k + 46k.
- tui (opus): `while`/`until` → keyword fixed; deliberate: the row counts as `watched` and `dir=` is the basename (both planned), `read` stays `loop-var`, the row keeps its machine form for the measure; 67k + 45k.

## Decision log

- Release after 2026-11-25, so that t22 counts the holds of one rule
  version (t23 Resolution, decisions#258).
- 2026-11-19, implementation: a heredoc body is read raw, so `classify`
  no longer sees a `docs(plan)` line written into a file by a heredoc (1
  backtest call), and sees the boundaries after an apostrophe-bearing body
  (67 calls). Both are more correct than "as today".
- Delta 1 (2026-11-19, after the first backtest): an `&& NAME=` with no
  earlier binding keeps its value (it only adds a check); a new
  `conditional` shape; with no `-C`, `here` is checked unless it is also
  the payload cwd; the bar restated as a share of the boundaries the rule
  sees.

<!-- panel: repos=amont-agent,decisions adds= reviewers=backend,lang:rust,tui,unix body-sha=c77bf067a21d -->
