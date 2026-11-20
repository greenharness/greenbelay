---
status: active
branch: fix/clear-boundary-keyword-prefix
repos: [greenbelay, decisions]
adrs: [ADR-0077]
programme: session-boundaries t25
plan-id: p-01M7V502RH18TB99BF7ZSHF4R0
---
# clear-at-boundary sees a boundary whose first word is a keyword

## Review panel

👉 **Decide:** none — approve if 21 loop boundaries (about 1.3%) are worth judging, with #298's lows closed.
📍 greenbelay, decisions · 2 phases · next: Phase 1, merged unreleased until t22 closes. Panel: backend, lang:rust, tui, unix (opus); codex not installed.
**Changed by review:** `command`/`builtin` keep `not-literal` before eval/source/.; heredoc and cap tests pinned; cap pilot read by row body.
**Verdicts:** round 1: 4 approve-with-changes, 11 applied, 1 duplicate; round 2: 3 approve-with-changes, 1 approve; 7 lows carried; words 1302 → 1549 (×1.19); remove 0, merge 0.
📄 Full reviews: [2026-11-20-clear-boundary-keyword-prefixed.reviews.md](2026-11-20-clear-boundary-keyword-prefixed.reviews.md)

## Picture

```
 today
 -----
 step "do git -C $H worktree remove w"
   --> classify --> git_call: first word "do" is not git --> no boundary

 after this plan
 ---------------
 step "do git -C $H worktree remove w"
   --> classify --> command_words strips "do"
       --> git_call --> worktree remove --> walk.directories
           |-- $H known     --> dirs [/h/repo], judged (may hold)
           `-- $r loop var  --> unresolved loop-var, journalled

 judge at the cap: save state.key --> a repeat Stop takes the key path
   --> one unresolved row, one cap row
```

A boundary step that opens with a keyword used to be skipped by
`classify`. After this plan it reaches `git_call`, as it does in the walk,
and the walk resolves its directories.

## Context

Ticket t25 (decisions `docs/programmes/session-boundaries/t25-keyword-prefixed-boundaries.md`),
filed from t23. `classify` in `crates/greenbelay-process/src/clear_boundary.rs`
hands the raw step words to `git_call`, which skips only leading `NAME=`
words. A step that starts with a reserved word, such as `do git -C $H
worktree remove …` or `then git commit -m "docs(plan): …"`, is therefore
never a boundary, even though the walk (`Walk::apply`) already strips the
same keywords. A rerun of t23's backtest over the transcripts to
2026-11-20 finds **21** such calls among the boundaries `boundaries()`
misses: `do` 12, `then` 8, `else` 1. All of them are `worktree remove` calls
inside a `for` loop or an `if`. None comes from a heredoc body. t23 counted
19 to 2026-11-18.

The ticket also asks to fix the F4b review lows that greenharness/greenbelay#298
left open, or to record them as declined.

## Goal

A boundary written after `then`, `do`, `else`, `elif`, `{`, `if`, `while`
or `until` is judged like any other: its `-C` words resolve through the
walk, a loop variable stays `unresolved loop-var`, and a name assigned
before the loop resolves. The #298 lows are closed in the same branch.

## Non-goals

- `!`, `time`, `for`/`select`/`case` arguments: the walk does not strip
  them, and no call in the transcripts uses them. One shared keyword set
  keeps `classify` and the walk from drifting apart.
- Marking a boundary inside a compound as unresolved. It ran, in the same
  way a boundary after `&&` ran. Its directory is the walk's, and a `cd`
  inside the compound is already `Err(Keyword)`.
- The release. This branch merges unreleased and rides the greenbelay
  release of #298–#301, which comes after t22's window closes on 2026-11-25
  (t23 Phase 2).

## Code review

- greenbelay: backend, lang:rust, unix, tui (the `unresolved` and `cap` journal rows are read in `explain`)
- decisions: none — markdown only

## Behaviour

| Command | Today | After |
|---|---|---|
| `for r in a b; do git -C $r worktree remove x; done` | no boundary | boundary, `unresolved loop-var`, dirs = cwd chain |
| `H=/h/repo; for w in a; do git -C $H worktree remove $w; done` | no boundary | boundary, dirs `[/h/repo]` (at `deny`, it can hold) |
| `LIVE=/h/r; if true; then git -C "$LIVE" worktree remove x; fi` | no boundary | boundary, dirs `[/h/r]` |
| `{ git commit -m "docs(plan): x"; }` | no boundary | plan commit |
| `if true; then git commit -m "docs(plan): x, done"; fi` | no boundary | no boundary (closing commit) |
| `echo "then git worktree remove x"` / a quoted `"do"` | no boundary | no boundary |
| `cat > s.sh <<EOF` body line `do git worktree remove x` | no boundary | boundary, as a bare `git worktree remove` body line is today (pinned by a test) |
| `command -v jq && git worktree remove ../x` (cwd `/h/a`) | `unresolved not-literal` | resolved, dirs as plain |
| `command cd /h/b && git worktree remove x` | `not-literal` | `not-literal` (unchanged) |
| `command -v cd && git worktree remove x` | `not-literal` | `not-literal` (conservative) |
| `builtin echo hi; git worktree remove x` | `not-literal` | resolved |
| `builtin source env.sh; git worktree remove x` | `not-literal` | `not-literal` (unchanged) |
| two Stops past the advice cap on one unresolved boundary | two `unresolved` rows, two `cap` rows | one of each |

## Phases

- [x] Phase 1 (done 2026-11-20): keyword-prefixed boundaries and the #298 lows. Files:
  `crates/greenbelay-process/src/clear_boundary.rs`, `CHANGELOG.md`.
  - **One keyword set.** Add `fn command_words(words: &[Word]) -> &[Word]`
    that skips leading bare `then`, `do`, `else`, `elif`, `{`, `if`,
    `while` and `until` words, the set `Walk::apply` strips before a
    simple command. `classify` calls it before it builds `texts`, so
    `call.directories` index into the stripped slice. `Walk::apply` keeps its
    own loop, because it also pushes and pops compounds, and draws its 8 openers
    from one `const` shared with `command_words`; the closers (`done`, `fi`,
    `esac`, `}`) and `for`/`select`/`case` stay as their own arms in `apply`. That way the two sets
    cannot drift apart. `keyword()`'s bare-quoting test is kept, so a quoted
    `"then"` is a word, not a keyword. A const slice cannot be a `match`
    pattern, so a test loops over the const and asserts that `classify`
    finds `git worktree remove x` after each word and that `apply` strips it.
  - **Low 1, the cap repeat.** At `advices >= MAX_ADVICES_PER_SESSION`,
    save `state.key = key`, with `advices`, `turn` and `pending` unchanged,
    before `watched("cap")`. A repeated Stop on the same boundary then takes
    the existing `state.key == key` path: no second `unresolved` row and no
    second `cap` row. This matters because the `unresolved` row is what
    t23 Phase 2's 7-day count reads.
  - **Low 2, `command` and `builtin`.** `command` and `builtin` look past
    their option words (`-p`, `-v`, `-V`, and `--` as end of options).
    Behaviour gains the row `command -- cd /h/b && git worktree remove x` → `not-literal`. When the next word is `cd`, `pushd`, `popd`, `eval`, `source` or `.`, they
    give `not-literal`, as today. Anything else changes nothing. `eval`,
    `source`, `.`, and a bare `pushd` or `popd` stay `not-literal`.
  - **Low 3, Rust points.**
    - Merge the two `Shape::Keyword` arms in `apply`.
    - `status_ok` takes `Option<&Word>`, with `.copied()` at its call site.
    - `Shape::as_str` gets a doc line (the journal row's shape name).
      `Display` is not added: `as_str` is the only reader.
    - The `entry_start` doc says "rooted", not "absolute".
    - The doc line on `Pending.unresolved` is already there (line 1639).
      It is recorded as done.
  - **Tests**, in the module's `tests`:
    - one test per Behaviour row, red on `origin/main` for the rows that
      change;
    - `two_stops_at_the_cap_write_one_unresolved_row` (asserts one `cap` row too);
    - `two_boundaries_at_the_cap_write_two_cap_rows`;
    - `the_cap_is_reported_when_the_file_start_is_not_reached` still
      passes.
- [ ] Phase 2: record in decisions. Files:
  `docs/programmes/session-boundaries/t25-keyword-prefixed-boundaries.md`,
  `docs/programmes/session-boundaries/t23-cd-and-same-command-variables.md`,
  `docs/programmes/session-boundaries/map.md`.
  - t25 gets its Resolution: the backtest numbers and each low with its
    outcome. Then close it.
  - t23's table gains two rows:
    - keyword-prefixed boundary, `-C` known: checked;
    - keyword-prefixed boundary, `-C` a loop variable: `unresolved loop-var`.
  - The map's Decisions line and Handoff are updated. The Handoff notes
    that the release of #298–#301 now carries t25 as well, so t23's
    7-day `unresolved` count covers loop boundaries too.

### 🧑 Your steps

None.

## Verification

- `cargo test -p greenbelay-process`, plus the workspace gate at the
  commit: green. The new tests are red on `origin/main` (checked by
  running them before the fix lands).
- `greenbelay corpus check`, with the branch binary: every reviewed Stop
  case and command case agrees, as before.
- **Backtest (ADR-0060).** Run t23's harness (`rows.py`, `t23_backtest.rs`
  as an example of `greenbelay-process`, `score.py`) on `origin/main` and
  on the branch, over the transcripts to 2026-11-20, and again with
  `UNTIL=2026-11-18` so the count can be compared with t23's 19.
  - **Bar:** the number of boundaries `found` rises by exactly the 21
    keyword-prefixed rows (19 to 11-18), with no other row gaining or
    losing a boundary.
  - Every call that is checked correctly today keeps its directories. t23
    counted 846. The rows Low 2 moves from `unresolved not-literal` to resolved are counted on both binaries, listed in t25, and each is read by hand; no other row changes its `unresolved` shape.
  - Each newly found row is read by hand.
  - The unresolved share is recomputed and recorded in t25. t23 had 7.3%,
    and its bar is 15%.
- **Piloted**, on the branch binary in a scratch `HOME`/state dir at
  `deny`, as t23's pilot did:
  - **dirty:** `P=<probe>; for w in x; do git -C $P worktree remove
    ../$w; done`, cwd elsewhere, probe modified. Expected: held, with the
    journal row `pending uncommitted changes dir=probe`. Control: the
    installed 6.5.0 sees no boundary.
  - **clean:** the same, with the probe committed and pushed. Expected: a
    Continue.
  - **loop variable:** `for r in <probe>; do git -C $r worktree remove x;
    done`. Expected: one `unresolved loop-var worktree remove` row.
  - **cap:** in a state already at the cap, two Stops on that
    loop-variable boundary. Expected, read from the scratch `journal.log`
    by row body (the tally counts both as `watched`, journal.rs:800):
    exactly one `unresolved loop-var worktree remove` row and one `cap`
    row, against two of each on `origin/main`. The `explain
    clear-at-boundary` watched total is noted beside it (2, against 4).

## Verification record (Phase 1, 2026-11-20)

| Check | Input | Expected | Observed |
|---|---|---|---|
| New tests | `cargo test -p greenbelay-process`, the three fixes reverted on a copy | red, then green | 5 of 6 new tests red with the fixes reverted. `two_boundaries_at_the_cap` passes on both, by design: it guards against over-silencing. All green at dcdeb386 and ff9f2e73 |
| Corpus | `greenbelay corpus check`, branch binary | every case agrees | `clear-at-boundary 20 reviewed (11 negative), 0 unreviewed, precision 100%` |
| Backtest (ADR-0060) | `rows.py` to 2026-11-20 (1,717 calls), `t23_backtest` on `origin/main` and on the branch | +21 found (19 to 11-18), none lost, dirs kept | +20 found to 11-20, +19 to 11-18 (matches t23's 19); lost 0; 0 rows lose a directory. The 21st regex hit is an inbox `printf` that quotes "do git worktree remove" inside single quotes, correctly not a boundary. 19 already-found calls gain a second, keyword-led boundary: 6 of them add a directory to the check (e.g. `/Perso/adr-lint`), and 2 add an `unresolved not-literal` shape from a later `$R` loop boundary. Each was read |
| Low 2 rows | the same, `not-literal` → resolved | listed and read | 1 row: `cd …-wt-work-board && command grep -n setup relais.toml …`, now resolved, which is correct |
| Unresolved share | the branch, all boundaries seen | ≤ 15% | 112/1,535 (7.3%) to 11-20; 108/1,383 (7.8%) to 11-18 (main: 99/1,364) |
| Pilot, dirty | scratch HOME at deny, `P=<probe>; for w in x; do git -C $P worktree remove ../$w; done`, cwd elsewhere, probe modified | held | journal `pending uncommitted changes dir=probe`, no output |
| Pilot, clean | same, probe committed and pushed | Continue | `{"decision":"block",…}`, journal `denied worktree remove` |
| Pilot, loop variable | `for r in <probe>; do git -C $r worktree remove x; done` | one `unresolved loop-var` row | `watched … unresolved loop-var worktree remove`, then `denied worktree remove` |
| Pilot, cap | state at `advices: 3`, two Stops on one boundary | 1 `unresolved` + 1 `cap` row | loop boundary: 1 + 1. Subshell boundary: branch 1 + 1; installed 6.5.0 (without #298's row) 0 + 2 `cap` |
| Control | installed 6.5.0, the dirty, clean and loop pilots | the bug | `no boundary` on all three |

## Implementation review

- implementation-review: approve-with-changes twice. Fixed: the exact keyword asserts, the record and the Phase 1 tick. Deliberate: brace-expanded wrapped words (contrived). 63k+33k, 79+47 s.
- backend: approve-with-changes, then approve. Fixed: an unreadable wrapped word stays unread; the exact asserts. Deliberate: `command -v "$tool"` stays unread (cautious). 68k+26k, 56+23 s.
- lang:rust: approve-with-changes, then approve. Fixed: `let _saved`, the asserts, the doc wording. 57k+26k, 61+21 s.
- unix: approve. Deliberate: `!`/`time`/`case`-arm prefixes (Non-goals); `command -v cd` stays cautious. 46k, 42 s.
- tui: approve. Fixed: the CHANGELOG wording and the keyword order. 50k, 37 s.

**Next:** Phase 2, the decisions record, after this PR merges.

## Decision log

- 2026-11-20: the keyword set is the walk's own (8 words), not a wider one:
  zero transcript calls use `!` or `time`. `command git …` and `if ! git …`
  also stay non-boundaries, though both run git; none seen in transcripts.

<!-- panel: repos=greenbelay,decisions reviewers=backend,lang:rust,tui,unix body-sha=c5a73903e303 -->
