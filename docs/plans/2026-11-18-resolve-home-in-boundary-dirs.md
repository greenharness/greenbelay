---
status: active
branch: fix/clear-boundary-home-dirs
repos: [amont-agent, decisions]
adrs: [ADR-0062]
programme: session-boundaries t21
plan-id: p-01M7QA24GT5JE1AGWCW1K0DN6B
---
# clear-at-boundary resolves `~` in a boundary's `-C` directory

## Review panel

👉 **Decide:** none. Approve if a `~` path that was never expanded is all t21 is, and a fixture fact stored per directory is honest evidence for `deny`.
📍 amont-agent, decisions · 3 phases · 1 release · next: Phase 1 red test. Panel: backend, lang:rust, tui, unix (opus-5-5), codex.
**Changed by review:** the fixture's `per_directory` variant (serde); expansion that follows quoting; an expanded word joins home and skips the cwd (codex).
📄 Full reviews: [2026-11-18-resolve-home-in-boundary-dirs.reviews.md](2026-11-18-resolve-home-in-boundary-dirs.reviews.md)
**Verdicts:** 3 approve-with-changes, tui approve; 11 carried (Full reviews, read before Phase 1); words 1006 → 1338 (×1.33, the high and unverifiable fixes exempt); remove 0, merge 0.

## Picture

```
 boundary  git -C ~/.local/share/chezmoi worktree remove …
 cwd       ~/Developer/Perso

 today   resolve(cwd, "~/.local/share/chezmoi")
           = ~/Developer/Perso/~/.local/share/chezmoi
           -> not a dir -> counted clean -> advised
 after   expand "~" with home first
           = ~/.local/share/chezmoi
           -> git status " M dot_zshrc" -> held pending -> silent
 replay  home = "~", facts.repositories.per_directory
           {"~/.local/share/chezmoi": "uncommitted changes"}
           looked up by resolved dir -> held -> nomatch
```

The rule already named the dirty checkout; it lost it by joining `~` to
the cwd. Expanding home first makes the same check find it.

## Context

t21 asked which checkouts the dirty-tree check must cover so that Stop
8145b6c0-1794667260 stays silent. The answer: **no checkout was missing.**
The boundary command was `git -C ~/.local/share/chezmoi worktree remove …`,
so `Boundary.dirs` already named the chezmoi source. But `chain`/`resolve`
(`crates/greenbelay-process/src/clear_boundary.rs:414-439`) join the word
to the cwd lexically, so `~` is never expanded. The joined path is not a
directory, and `check_repositories` counts a missing directory as clean.

Primary evidence, from the original transcript
(`8145b6c0-f4ed-4e98-836a-ea64ce7213a0.jsonl`): the turn's own check of
that checkout printed `live: uncommitted changes; left alone` and ` M dot_zshrc`.
The last message asked the person to restore the file. So with `~`
expanded, the live rule would have found the checkout dirty and held the
boundary.

Fixture scan (16 Stop cases) of boundary `-C` words:
- `~/…`: 8145b6c0 (the `?` case), 52e82064 and e2fb2812 (`match`),
  32290a5c-1794907585 (`nomatch`). After the fix, the live rule really
  checks these. The corpus cannot show a live change, because those
  fixtures freeze the git facts as clean.
- `"$LIVE"`: 72e46887-1794943716 (`match`) and 1dbedd97 (`nomatch`). The
  Bash tool does not carry shell variables between calls, so an unset
  `$LIVE` is empty, and `git -C ""` is the cwd, which the check already
  covers. These stay as they are.
- relative or `.`: unchanged.

## Goal

- A boundary's `-C` directory starting with `~`, `~/`, `$HOME` or `${HOME}`
  resolves against the home directory before the cwd join. The home
  directory is `$HOME` live. Any other `$VAR` or `~user` word keeps
  today's handling.
- The Stop fixture can freeze the repositories fact per directory, so the
  replay asks for the directories the rule actually resolved. With home
  taken as a literal `~` in replay, the keys match the exporter's scrubbed
  paths (`clear_boundary_export.rs:394`, `Scrub::text`).
- 8145b6c0 gets the label `nomatch`, and `corpus check` agrees with all 20
  lines. Then `greenbelay graduate clear-at-boundary --to deny` is run
  again, without `--force`, on the released, installed binary.

## Non-goals

- Widening the check to every repository the turn touched (Edit paths,
  read-only `git -C`). That would hold boundaries over the person's own
  unrelated dirty checkouts, and the frozen-clean corpus cannot measure
  it.
- `cd X && git …` (a `cd` in the same command as the boundary) and
  `VAR=path; git -C "$VAR"` in one command. Filed as a follow-up ticket
  in the programme inbox, not built here.

## Phases

### Phase 1: expand home in `-C` directories

- `chain(cwd, home, directories)` and `resolve` (`clear_boundary.rs`):
  - Resolve an eligible leading `~`, `~/`, `$HOME` or `${HOME}` by
    joining its relative suffix directly to `home`, bypassing the cwd
    join even when replay supplies the symbolic home `~`. Join only
    unexpanded relative words to cwd, then fold `.` and `..` as today.
    Validate live HOME before injection; replay's explicit `Some("~")`
    bypasses that environment validation. Pin both the historical fixture
    path and `-C ~/a -C ../b` with symbolic home in unit tests. `segments` records each word's leading quoting: a
    `~` expands only when unquoted, `$HOME`/`${HOME}` only when not
    single-quoted (POSIX XCU 2.6.1, 2.2.2); a quoted word keeps today's
    handling.
  - `home: Option<&Path>`. With `None`, the word is kept as written
    (today's behaviour). An unset, empty or non-absolute `$HOME` gives
    `None`. Only whole-word `~`, `$HOME`, `${HOME}` followed by the end or
    `/` match; `$HOMEX`, `${HOME}x`, `~+`, `~-`, `~user` are kept.
- `boundaries(window, cwd, home)` passes `home` through `classify`.
  Callers:
  - `judge` takes home from a new `StopView.home`, filled from `$HOME` in
    `StopView::of`.
  - the exporter (`clear_boundary_export.rs:585`) passes the same home its
    `Scrub` holds.
  - the unit tests pass `None` or a fixed path; only `StopView::of`
    reads `$HOME`, and one test covers it.
  - A `Pending.dirs` saved by the old binary keeps the joined path for
    one resume; the next person prompt clears it
    (`clear_boundary.rs:1263`). No migration; one test pins it.
- Unit tests (red first):
  - `boundaries` on the 8145b6c0 fixture's entries, with home `~`, yields
    dirs `["~/.local/share/chezmoi"]`. Today it yields
    `~/Developer/Perso/~/.local/share/chezmoi`.
  - `resolve` cases: `~`, `~/a`, `$HOME/a`, `${HOME}/a`, `~user/a`
    (kept), `$OTHER/a` (kept), `"~/a"` (kept), `'$HOME/a'` (kept),
    `"$HOME/a"` (expanded), `-C ~ -C rel` chaining.
  - `check_repositories` against a `TestRepo` reached through a `~/…`
    word, using a temp dir as home: NotClean when a file is modified.

### Phase 2: per-directory repositories fact

- `RepositoriesFixture` gets a third variant `PerDirectory(BTreeMap<String, String>)`, written `{"per_directory": {...}}` in JSON (the enum is externally tagged; a bare object matches no variant), mapping directory (as
  resolved in replay) to a verdict: `"clean"`, `"timed_out"`, or a
  not-clean reason. The string forms keep their meaning, so the other 15
  fixtures stay unchanged.
- In `StopInputs`, the replay closure becomes `|dirs| …`. For the map
  form, it walks `cwd` + `dirs` in order the way `check_repositories`
  does: the first not-clean or timed-out entry wins, and a directory the
  map does not name is clean. Keys are resolved directories, not git toplevels (live reports the toplevel and skips non-directories); the corpus header says so. `replay_at_deny` takes the closure instead
  of a frozen `Repositories`.
- `StopView.home` is `Some("~")` in replay.
- The 8145b6c0 fixture: `facts.repositories =
  {"per_directory": {"~/.local/share/chezmoi": "uncommitted changes"}}`, with the transcript
  line above cited in the cases comment.
- `clear-at-boundary.cases`: `?` → `nomatch` for 8145b6c0, with a `# why:`
  line that replaces the old "stays `?` until the dirty-tree check is
  widened" comment and names path resolution as the cause. The corpus header notes that a map fact names a directory the
  transcript showed dirty.
- Tests:
  - a map fact whose key the rule does not resolve (the pre-fix path)
    replays as clean. This proves the case passes only through the
    expansion.
  - `corpus check` passes on all lines.

### Phase 3: release and graduate

- Release greenbelay 6.4.1 (fix) through the `tag-release` skill
  (`Cargo.toml`, `CHANGELOG.md`), and install it on this machine.
- decisions files: `docs/programmes/session-boundaries/t21-dirty-tree-beyond-the-boundary.md`,
  `t10-advise-or-deny.md`, `map.md`, and the new follow-up ticket.
- Run `greenbelay graduate clear-at-boundary --to deny` without `--force`
  on the installed binary, run bare (not through `tail`/`head`), and
  record its output and exit code.
- decisions, one PR:
  - t21 Resolution: the cause was path resolution, not coverage. Record
    the fixture scan and the graduate result.
  - t10: a line saying the deny is applied, or why it was not.
  - the map Handoff.
  - the follow-up ticket for `cd`/same-command variables.

## Code review

- amont-agent: backend, lang:rust, unix (tui left out: no output a person reads changes; the note text is unchanged)
- decisions: none — markdown only

## Verification

- Live, on the installed binary: feed the hook a Stop whose boundary is
  `git -C ~/.cache/cab-probe worktree remove x`, where that repository
  has a modified file. Expected: journal `pending uncommitted changes`,
  no Continue. Commit the file and repeat. Expected: Continue.
- The red test fails on `origin/main`, and passes after Phase 1.
- `greenbelay corpus check` reports 20 Stop/command lines agreeing, with
  no `?` left for clear-at-boundary.
- Live: after install, `greenbelay graduate … --to deny` passes. The
  `-C ~/…` boundaries in the next 7 days are counted: the journal's
  `pending <reason>` holds do not name the directory
  (`clear_boundary.rs:1226`), so take the holds from the journal, keep
  those whose boundary command in the transcript has a `-C` word starting
  with `~` or `$HOME`, and read each against its transcript to confirm the
  checkout was really dirty. Record the count of holds, the count selected
  and the count confirmed (due 2026-11-25, recorded in t21). Pass: 0
  holds where the dirt was the person's own unrelated edits. One or
  more: `greenbelay graduate clear-at-boundary --to advise` and reopen
  t21.

## Verification record

Phases 1 and 2 done on d0eaba67; Phase 3 is next.

- The branch binary in a scratch HOME at `deny` (the installed-binary
  run follows the release). The boundary is
  `git -C ~/.cache/cab-probe worktree remove x` at 250k tokens. The repo
  has an upstream, and 0 commits ahead.
  - Modified file → expected a hold, no Continue. Actual: silent, and the
    journal reads `deny watched … pending uncommitted changes`.
  - The file restored, not committed → expected a Continue. The plan's
    line says "commit the file", but a commit leaves the branch one ahead
    of its upstream, and the check holds that as unpushed (codex#3,
    carried). The restore is the clean state the line means. Actual:
    `{"decision":"block",…clear-at-boundary…}`, and the journal reads
    `deny denied … worktree remove`.
  - Control: installed 6.4.0 with the file modified → it denied (the bug).
- Red first: with home ignored in `chain`, 5 of the new tests fail,
  including `the_8145b6c0_boundary_names_the_chezmoi_source`. With the
  fix, all 56 `clear_boundary` tests pass.
- `cargo test -p greenbelay-process` passes 322 tests; `-p greenbelay-guard`
  passes 867; `--test corpus_stop` passes 5. The commit gate's workspace
  suite passed on d0eaba67.
- `greenbelay corpus check` (branch binary): `clear-at-boundary 20
  reviewed (11 negative), 0 unreviewed, precision 100%`.
- The word table was checked against zsh 5.9 (`HO=x; print -rl -- …`).
  Every case agrees except `${HOME}x`: zsh prints `/hx`, a sibling of
  home. The rule keeps it as written, like any word it does not resolve.

## Decision log

- Carried review items, as implemented:
  - `Word` keeps one `Quoting` per character, so `$HO"ME"/a`,
    `"\$HOME/a"` and `\~/a` stay literal.
  - `segments` stays a text-only wrapper over `shell_words`, so
    `adds_worktree` and `commit_subject` are unchanged and only
    `classify` reads quoting.
  - An expanded word replaces the chained base.
  - A `..` above the symbolic `~` folds to an empty path (pinned).
  - The probe used a baseline with an upstream and a restore, run at
    `deny`.
- `git_call` returns the positions of its `-C` words rather than their
  text, so `classify` can look up each word's quoting.
- Implementation review round 1, fixed:
  - the `home_from` parameter is renamed `home_variable`;
  - the exporter's home goes through `home_from`, so there is one filter;
  - a `per_directory` fact is parsed into `DirectoryFact` when it loads,
    and refused when its key is neither absolute nor under `~`, or its
    reason is not one the check gives;
  - a test covers how `StopView::of` reads `$HOME`.
- Deliberate:
  - `classify` still clones the word texts for `git_call`, so
    `adds_worktree` keeps a text-only path;
  - `StopView::of` reads `$HOME` as the plan says, since it is the
    hook's input adapter.

<!-- panel: repos=amont-agent,decisions reviewers=backend,lang:rust,tui,unix,codex body-sha=4b84852b29c0 -->
