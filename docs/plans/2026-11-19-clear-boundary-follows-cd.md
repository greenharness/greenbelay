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
📍 amont-agent, decisions · 2 phases · 1 release after 2026-11-25 · next: Phase 1. Panel: backend, lang:rust, tui, unix (opus), codex.
**Changed by review:** the walk starts at the call's own cwd; conditional and compound assignments go Unknown; redirects, unassigned names and `dir=` holds handled.
**Verdicts:** 4 approve-with-changes; codex rework (3 high applied), then backend approve-with-changes; 6 carried; words 1428 → 2369 (×1.66, high findings exempt); remove 0, merge 0.
📄 Full reviews: [2026-11-19-clear-boundary-follows-cd.reviews.md](2026-11-19-clear-boundary-follows-cd.reviews.md)

## Picture

```
 cwd       ~/Developer/Perso/amont-agent-wt-rel-641          (payload)
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
`export`, `unassigned`, `not-literal`.

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

### Phase 1: walk the command in order

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
    `&` (it runs in a subshell), next to `||` (it may not have run), or
    last in a pipeline (zsh runs it in the current shell) sets
    `here = None`. An assignment ended by `|` or `&`, or after a `|`,
    sets `Unknown`. An assignment or `cd` that follows a `&&` or `||` is
    conditional: it holds for the rest of that `&&`/`||` list, and when the
    list ends (`;`, newline, `&`) the names it set become `Unknown` and a
    conditional `cd` sets `here = None`. So `R=/a; false && R=/b; git -C
    "$R" …` is unresolved, never `/b`. A first word that
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
  `cwd`. With no `-C` and `here != cwd`, `dirs = [here]`. When `here` or
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
- Backtest (ADR-0060): run the branch `boundaries()` over the 1,545
  commands of decisions#258 before merging. Expected: at least 400 of
  the 421 followable commands resolved to the directory the command acted
  on, and each of the 994 calls today already checks correctly keeps that
  directory; each disagreement is listed in t23 with the counts by shape.
  Below that bar, hold the merge and return the plan to review.
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

## Decision log

- Release after 2026-11-25, so that t22 counts the holds of one rule
  version (t23 Resolution, decisions#258).

<!-- panel: repos=amont-agent,decisions adds= reviewers=backend,lang:rust,tui,unix body-sha=d6b672196a41 -->
