---
status: done
branch: feat/bash-read-dumps
repos: [greenbelay]
adrs: []
plan-id: p-01M80DK0NSNVT14SHGAM821DTM
---
# Bash reads: close the `whole-file-dump` gaps (sed/cat/grep)

## Review panel

👉 **Decide:** none. Approve if the bet is acceptable: Phase 0's replay decides (b); (a) ships regardless, because it is a bug.
📍 greenbelay · 3 phases · 1 release · next: Phase 0 replay. Panel: backend, lang:rust, tui, unix (opus-5-5), codex.
**Changed by review:** per-path refuse-once keys, so a subset retry passes; `examine` widened to bounded windows; `||` and FIFO safety.
**Verdicts:** 4 approve (rust rework in round 1; Codex rework, three findings applied); 14 carried; words 1252 → 1792 (×1.43, from blocking/high findings and fixes to wrong statements); remove 0, merge 0.

📄 Full reviews: [2026-11-22-bash-reads-whole-file-dump-gaps.reviews.md](2026-11-22-bash-reads-whole-file-dump-gaps.reviews.md)

## Picture

```
 today                               after this plan
 -----                               ---------------
 cat a; cat b (each < 8 KB)          cat a; cat b (sum > 8 KB)
   --> one file judged --> passes     --> every dump judged --> refused once
 sed -n 1,50p log.jsonl (40 KB)      sed -n 1,50p log.jsonl (40 KB)
                                     [Phase 2, if Phase 0 keeps it]
   --> 50 lines --> advise only        --> window bytes --> refused once
```

A command's reads used to be judged one file at a time and by line count.
After this plan, they are judged by the bytes the whole command pours into
the context.

## Context

The rtk evaluation (2026-11-22, `greenbelay output-audit` command families, #310) named Bash
`sed`/`cat`/`grep` reads as the next lever: 10.6 MB of main-conversation output and 25 MB of
subagent output over 11-07..11-21. The proposal was "extend `read-unbounded-large` /
`whole-file-dump` to Bash reads".

`whole-file-dump` already is the Bash sibling of `read-unbounded-large`. It parses
`cat`/`head`/`tail`/`sed`/`nl`/`bat`/`less`/`more` (`crates/greenbelay-guard/src/rules/dump.rs`).
It has been at deny since 11-09. It refuses once a whole dump of one regular file over 8 KiB, and
it advises on everything else. The refusal works: 94 subagent results were refused between 11-09
and 11-21, and only 2 were repeated unchanged.

A rough bucketing of 11-09..11-21 transcripts (my script, not the real parser) shows where the
bytes are. Most of the 25 MB is legitimate windowed reading: `sed -n` windows under 100 lines are
6.1 MB over 1,510 subagent calls, about 4 KB each. What is left is the remaining gaps, an upper
bound of about 5 MB per two weeks in subagents and about 1 MB in main:

- **(a) Aggregation, a correctness bug.** `examine` returns only the first matching dump, and
  `confirm` judges that one file. So `cat small.md && cat big.rs` is never refused on `big.rs`.
  Several small files in one command (`cat a b c`, `cat a; cat b`) pass however much they add up
  to. About 110 subagent commands over 8 KB passed this way.
- **(b) Bounded windows are judged by line count, not bytes.** `sed -n 'X,Yp'`, `head -N` and
  `tail -n N` only ever advise. A 50-line window over a long-line file (jsonl, minified code)
  returns more than 8 KB, and the Read tool would have cut those lines at 2,000 characters. That
  is about 3 MB of subagent results over 8 KB.
- **(c) grep: deferred.** A grep's output size cannot be known before it runs. Advising on its
  shape (recursive, no `-l`/`-c`/`-m`/`| head`) would be wrong about 90% of the time: 27 of 237
  unlimited recursive subagent greps returned over 8 KB. Out of scope for this plan; reopen only
  if Phase 0 finds a shape that predicts size.

All of these figures are upper bounds. My script charged a command's whole output to its first
reader, and it counted heredoc writes. Phase 0 re-measures them with the real parser before any
threshold is fixed.

Reviewer agents have no Bash tool (implementation-review, code-review-*, plan-review-*,
relais-review: Read/Grep/Glob/LSP only), so no reviewer exemption is needed.

## Phases

- **Phase 0: replay with the real parser (gate)** ✅ done 2026-11-22: (b) dropped, (a) ships
  Files: `crates/greenbelay-guard/src/backtest.rs`.

  - Measure with `greenbelay backtest`, using `dumps(parsed)`, charging each call its recorded tool_result byte length (stat only to class whole vs bounded: worktrees of 11-09..11-21 are mostly gone). If backtest
    carries neither confirm nor result bytes, add a `--bytes` flag for `whole-file-dump` in
    `crates/greenbelay-guard/src/backtest.rs`, with the same parser and no new one. First check the in-flight
    `greenbelay-wt-replay-label` branch (`feat/backtest-replay-label`) so the work is not
    duplicated.
  - Output for 11-09..11-21, main and subagents apart: the calls and result bytes that (a) and (b)
    would refuse, counting only attributable calls: a dump that is the command's only output, its
    result complete. Mixed commands, truncated or refused results, and calls whose files are gone are
    reported apart, and the kill criteria apply to the attributable count alone.
  - **Kill criteria:** drop (b) if it would refuse less than 1 MB of subagent results in the window,
    or if over 25% of its refusals are files whose average line is 2,000 characters or less (Read would not have cut them). Ship (a)
    regardless, because it is a bug.
  - Record the numbers in the rule's `Evidence` and in this plan's outcome section.

- **Phase 1: (a) judge every dump in a command** ✅ done 2026-11-22 (relais run-660e3c923aee7-12ead, verified by run-660e456e5476e-5f97)
  Files: `crates/greenbelay-guard/src/rules/whole_file_dump.rs`, `src/hook.rs`, `tests/hook.rs`.


  - `examine` keeps its shape and span, so the corpus judgements of the pure shape stay valid. Its
    span now covers the first through the last qualifying dump.
  - `confirm` iterates `dumps(ctx.parsed)`, not only the dump at `span.start`. It resolves each path
    with the existing `target` logic, split into `resolve(cwd, dump)`, and keeps the regular files.
    A command is refusable when the sum of its **whole** dumps of regular files exceeds `REFUSABLE`
    (8 KiB). A dump on the right of `||` is left out of the sum, and the left operand, which always
    runs, is counted: a reader that fails can still print before its fallback runs (`cat aa missing
    || cat bb` prints both), so the right side is uncertain, never zero. Do not reuse
    `runs_unconditionally` (dump.rs:198) for this: it also discounts the right of `&&`. Order: non-regular, missing and exempt files are left out of the sum
    (a missing operand is skipped, and the rest are still summed); refuse when the sum exceeds
    `REFUSABLE`; else the small (<4 KiB, now judged per file, not on the first only) and advise
    rules apply. `confirm` rebuilds the
    reason, remedy and excerpt from the refused files, not from `examine`'s first dump: each file with
    its KB (at most 5, then "and N more"), "together N KB, over the 8 KB…", and the remedy names
    the largest file. The hook's refuse-once sentence (`src/hook.rs:886`) becomes "If you need this
    output in the result, repeat…" in this phase, so it holds when Phase 2 is dropped.
  - Refuse-once: a new `OnceTarget::Files { paths, cwd }`, with one told key per normalised path (today's key shape). Call `lsp::told` for every path, never through a short-circuiting `any` (it records on `First`, lsp.rs:266-289); refuse when any answer is `First` and none is `Unrecorded` (an `Unrecorded` one journals `unrecorded` and advises), so repeating the command, or retrying with a subset (`cat big` after `cat big small`), passes. `settled` (hook.rs:835) is checked per path. Each path is resolved against `ctx.cwd_at(dump.at)`, not one shared cwd.
  - Exemptions: plans, diffs/patches and persisted output (not media: `cat x.png` pours raw bytes, unlike Read), for the refusal only. Exempt files are left out of the sum. A `cat docs/plans/x.md` is a deliberate whole read, as it is for Read.
    Those files still get the advice.
  - Regression tests: `cat small && cat big` refuses on `big`; `cat a b c` with each file under
    8 KiB and the total over it refuses; repeating the command passes; the reason names `big` and the total and the remedy does not name `small`; an exempt plan only advises;
    `cat fifo small.md` only advises and `cat fifo big.rs` refuses on `big.rs`, and the FIFO is never
    opened; a missing operand is skipped and the rest are summed; `cat a b c` repeated passes with all three keys recorded; `cat a || cat b` (each under 8 KiB) only advises; `cat big.rs || true` still refuses on `big.rs`.

- **Phase 2: (b) bounded windows by bytes (only if Phase 0 keeps it)** ❌ dropped by Phase 0's kill criterion
  Files: `crates/greenbelay-guard/src/rules/dump.rs`, `crates/greenbelay-guard/src/rules/whole_file_dump.rs`, `tests/hook.rs`.

  - For `Extent::Lines(n)` and `Extent::Bytes(n)`, and only after `metadata.is_file()` (never open a
    FIFO or device), measure the window's real size. Open with `O_NONBLOCK`, then `fstat` the handle
    and require `is_file()` again, so a path swapped for a FIFO after the stat cannot block the hook:
    - `sed -n 'X,Yp'` and `head -N`: a `BufReader` that counts bytes up to line Y, measuring the whole window
      (bytes and lines, skipped lines included) under a 1 MiB scan budget for the whole command; past
      the budget it only advises. sed without `-s` numbers lines across its operands as one stream, so a
      multi-file sed window is measured over the concatenation; `head`/`tail` are measured per file.
    - `tail -n N`: read backward from the end in 64 KiB blocks, capped at 1 MiB, so a large log costs
      a bounded read.
    - Files over 16 MiB are not opened, and only get advice.
    - `head -c N` / `tail -c N`: the window is `min(N, len)` from the stat; nothing is opened.
  - Refusable when the window is over `REFUSABLE` **and** its average line is over 400 bytes (the
    long-line case Read truncates). An ordinary 300-line code window stays advise. That part is the
    existing `MANY_LINES` advice, left as it is.
  - The reason for this case: "lines X–Y of `f` are N KB (average line M bytes; Read cuts lines at
    2,000 characters)", never "poured whole" (the hook's refuse-once sentence was changed in Phase 1).
  - The remedy for this case: Read with `offset`/`limit` (lines cut at 2,000 characters), or
    `cut -c1-300` / `jq` for jsonl.
  - `examine` now fires on every bounded window (today only `Lines(n) >= 100`, whole_file_dump.rs:48-52), and `confirm` returns `No` when the window is under `REFUSABLE` and under `MANY_LINES` lines (a 100+ line window under 8 KB keeps today's advice); the `a_window_a_pipe_or_an_edit_is_silent` test and the affected corpus cases are updated.
  - The window's start and end come from parsing the existing `Dump.window` (`start:count`, `tail:N`), not a new field.

## Non-goals

- grep (reason in Context (c)).
- Main-conversation-only thresholds. One threshold serves both, as for Read.
- Changing `read-unbounded-large`.

## Verification

- `cargo test --workspace` locally before each land. The memory lesson: the pre-commit gate runs
  no full suite, and threshold changes broke 7 `tests/hook.rs` tests last time. Update the
  `tests/hook.rs` cases around the `DUMP_DENY` fixture (lines ~4105–4271) that assert a bounded
  dump only advises. Phase 1 must leave them passing. Phase 2 adds a long-line refusal case beside
  them.
- `greenbelay corpus check`: every reviewed `whole-file-dump` case still judged the same. Add
  corpus cases for the aggregate and long-line shapes.
- `greenbelay check 'cat a.md && cat big.rs' --cwd <scratch>` and the long-line `sed -n` case, run
  by hand against scratch files with a fixed session id: expected `deny` naming both files and the total KB on the first call, `allow` and a journal `allowed-after-feedback` on the repeat.
- After release and install: a 7-day count from the journal of `whole-file-dump` `deny` /
  `allowed-after-feedback` outcomes, compared with Phase 0's prediction; within ±30%, else open a ticket. The ticket is due 7 days
  after install.

## Code review

- greenbelay: backend, lang:rust, unix, tui (the refusal text changes)

## Decision log

- 2026-11-22, Phase 0 placement: the replay is a dump class in `greenbelay output-audit`, which already
  pairs each Bash call with its result, not a `backtest --bytes` flag. Backtest replays `examine`
  with no results, and the in-flight `feat/backtest-replay-label` branch is editing `backtest.rs`.
  The parser is the same (`shell::lex` + `rules::dump::dumps`).
- 2026-11-22, Phase 2 dropped: the long-line windows it would refuse come to about 80 KB in the window,
  far below the 1 MB bar. The large windows are ordinary short-line reads of 100+ lines. Refusing
  those would only move them to a Read with the same `limit`, which `read-unbounded-large` does not
  refuse, so nothing would be saved. The carried items that belong to Phase 2 (3, 4, 8, 9, 10) go
  with it.

- 2026-11-22, `Evidence` stays at its 2026-09-09 measure: it counts the rule's fires per 1,000 calls,
  and Phase 0 measured result bytes by dump class, a different quantity. The counted target below
  carries the new measure.
- 2026-11-22, the refusal text keeps "8 KB" (deliberate, tui#4): every other greenbelay refusal and
  `read-unbounded-large` say KB, and 8 KB there means 8,192 bytes too.

## Target (ADR-0079)

- Cause: `whole-file-dump`, the dumps this change starts refusing: `whole-many` and `mixed` Bash dump
  results over 8 KiB (`output-audit` dump classes, `large`, main + subagents).
- Baseline, complete week 2026-11-09..2026-11-16: 55 events (2 main, 53 subagents) in 4,749 dump
  results. Rows saved: `~/.claude/greenbelay/attestations/bash-read-dumps-baseline/2026-W46.json`.
- Ships in week 2026-11-23 (excluded from the comparison).
- Recount: `greenbelay output-audit --since <week start>Z --until <week end>Z --json` over complete
  weeks from 2026-12-07, summing `large` over the `whole-many*` and `mixed*` keys of `main_dumps`
  and `sidechain_dumps`.

## Outcome

Phase 0 (`greenbelay output-audit --since 2026-11-09T00:00:00Z --until 2026-11-22T00:00:00Z`, the
dump classes), results over 8 KiB, calls/bytes:

| class | main | subagents |
|---|---|---|
| whole-many (attributable) | 0 / 0 | 22 / 325 KB (8 refused) |
| whole-many (mixed output) | 8 / 89 KB | 62 / 842 KB |
| mixed whole+window (attributable) | 2 / 21 KB | 15 / 210 KB |
| mixed whole+window (mixed output) | 10 / 110 KB | 47 / 616 KB |
| window, long lines (all) | 3 / 45 KB | 3 / 37 KB |
| window, Read would cut | 0 | 0 |

(a) is bounded by about 0.55 MB attributable and 2 MB in all over 13 days, mostly in subagents. (b)
is about 80 KB, so it is dropped.

### Verification record (Phase 1)

| input | expected | actual |
|---|---|---|
| `cargo test --workspace` (relais verify, amont cargo-test) | pass | pass, 718 s |
| clippy `-D warnings`, `cargo fmt --check` | clean | clean |
| `greenbelay check 'cat small.md && cat big.rs'` (1 KB + 20 KB) | refuses, names both files and the total | deny: `small.md` (1 KB), `big.rs` (20 KB), "Together 21 KB"; the remedy names `big.rs` |
| `greenbelay check 'cat a.txt b.txt c.txt'` (4 KB each) | refuses on the sum | deny, "Together 12 KB" |
| CI `tests (windows-latest)` on PR #314, first push | pass | fail: the symlink test used `std::os::unix`; gated `#[cfg(unix)]` (ff2e974e) |
| `greenbelay check 'cat big.rs \|\| true'` | refuses on `big.rs` | deny |
| `greenbelay check 'cat a.txt \|\| cat b.txt'` (4 KB each) | at most advises | silent: each file is under the 4 KiB small floor |
| repeat passes, subset retry passes, one journal outcome per command | as specified | covered by the new `tests/hook.rs` cases (`at_deny_the_whole_dumps_of_a_command_are_summed`, `at_deny_a_subset_retry_passes`) |

## Implementation review

- implementation-review: approve (round 3, after 2 rework passes). Fixed: names, two flag arguments, an out-parameter, two tests that could not fail, aggregate corpus cases, target line. Deliberate: no `Unreadable` test (a miss, never a wrong refusal). ~58–97k per round.
- backend: approve-with-changes. Fixed: an unread `||` caps at advice, a capped and quoted journal excerpt, a mixed told-set test. Deliberate: `Unreadable` untested, keys spent on a mixed set (panel-accepted), two test-id nits.
- lang:rust: approve. Fixed: no `expect`, typed `DumpClass`, private `resolve`, `Seen` keeps the charged length, `remedy_for` takes the files. Deliberate: `seen`'s tuple bool, a test's module, `Display` padding.
- tui: approve. Fixed: an 80-column table, a multi-file remedy naming the largest file, consistent rounding, `persisted` noted as JSON only. Deliberate: KB wording, the `refus` header, "+N"/"and N more".
- unix: approve, no findings (pass carried by ancestry after the rebase delta).

## Delivery

One branch and one PR in greenbelay (`worktree-task`), with a commit per phase. Release with
`tag-release` (minor). Install it on this machine and run the 7-day observation.

<!-- panel: repos=greenbelay reviewers=backend,lang:rust,tui,unix,codex body-sha=665edd2812bb -->
