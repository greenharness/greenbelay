---
status: done
branch: feat/measure-fingerprint
repos: [greenbelay, decisions, blue-factory]
adrs: []
plan-id: p-01M7Y99CKYX3AHH6GFTRZEKTTS
---
# A shared fingerprint for dated measures

## Review panel

👉 **Decide:** none — approve if one stdlib helper plus a sidecar file (not front matter, which relais refuses) is enough for dated measures.
📍 greenbelay, decisions, blue-factory · 5 phases · next: Phase 1 via relais. Panel: backend, lang:python, lang:rust, tui, unix (opus-5-5), codex.
**Changed by review:** stamp written before the `.txt`, `--sum` pairs both ways (unpaired = exit 1); two locks, so no flock self-deadlock; stored fingerprint verified against its recipe.
📄 Full reviews: [2026-11-21-shared-fingerprint-dated-measures.reviews.md](2026-11-21-shared-fingerprint-dated-measures.reviews.md)
**Verdicts:** final 3 approve, 2 approve-with-changes; codex 3 high applied; 14 carried; words 1282 → 1861 (×1.45, high/blocking fixes exempt); remove 0, merge 1.

## Picture

```
 today                            after this plan
 -----                            ---------------
 run --count--> number            run --count--> number + stamp
                  |                                 |
 compare <--------'               compare --fingerprint differs?
   (old recipe, unseen)                 |               |
                                       no              yes
                                        |               |
                                     number      refused (exit 3)
                                                 unless --rebaseline

 t09: daily.py --> <day>.txt + <day>.fp.json (never refused)
      daily.py --sum --> compare(*.fp.json) --mixed?--> refused (exit 3)
                                                        unless --rebaseline
```

A dated count used to be compared to a number made by another recipe
unseen; after this plan each count carries a fingerprint stamp, and a
mismatch is refused unless `--rebaseline` records both.

## Context

blue-factory t19, from t15's resolution. Several dated checks broke when
their counting recipe changed under them (t09's denominator, t12's journal
rotation, B1's recomputed baseline), and every break was found by hand. t15
decided: fingerprint the measures that are re-run over weeks and compared
to a number. The fingerprint hashes the script blob, the tool version, the
source id and schema marker, the filters and the window rule, never the
data. Coverage sits beside it, not in it. A different fingerprint is
refused unless `--rebaseline` records both.

Checked first, as the ticket asks: `relais programme check` refuses an
unknown front-matter key (`relais/crates/relais/src/programme/parse.rs:112`
allows only type, mode, status, blocked_by, repos, due). So the fingerprint
cannot live in ticket front matter. It lives in a sidecar JSON file beside
the measure's output, and the ticket body names that file.

## Goal

A measure re-run over days or weeks refuses to compare against a number
counted by a different recipe, and says which recipe fields changed. One
stdlib helper does it for every such measure. t09's daily recount and
greenbelay's `tools/read-rate.py` are the first two users.

## Non-goals

- No invariants such as "null rows = 0": t15 says a shrinking denominator
  or a recorder bug (B9) needs those, not a fingerprint.
- No wiring into the other three rate scripts, t12 or B1 `stats --time`
  in this plan. They adopt it when next re-run.
- No `fingerprint` front-matter key in relais.
- No change to `count-context.py` itself (its pinned blob 16a5788d stays).

## Code review

- greenbelay: backend, lang:python, unix (tui left out: no interactive output, one table and one stderr line; architect left out: a standalone stdlib script, no module or contract another crate consumes)
- decisions: none — a docs note in one ticket
- blue-factory: none — ticket resolution and map handoff

## Behaviour

`tools/measure_fingerprint.py` (stdlib only, importable and runnable):

- `recipe(script, tool_version, source, schema, filters, window)` returns
  a dict. `script` is a path; it is stored as its git blob id (sha1 of
  `blob <n>\0<bytes>`, computed in Python, no git needed), so it matches
  the blob ids tickets already cite.
- `canonical(recipe)` is `json.dumps(sort_keys=True,
  separators=(",", ":"), ensure_ascii=False)`; `fingerprint(recipe)` is
  `sha256:<hex>` of its UTF-8 bytes.
- `check(path, recipe, coverage)`, and `rebaseline(path, recipe, coverage)`
  for the accepting case (no flag argument):
  - no file: writes `{"fingerprint", "recipe", "runs": [run 1], "rebaselines": []}`
    as the baseline;
  - same fingerprint: appends `{"at", "coverage"}` to `runs`;
  - different, through `check`: raises `RecipeRefused`, whose message names
    the changed recipe keys with old → new values, then the sidecar path and the exact command to accept it (the same argv plus `--rebaseline`). Nothing is written;
  - different, through `rebaseline`: appends `{"at", "from": {fp, recipe},
    "to": {fp, recipe}}` to `rebaselines`, makes the new recipe the
    baseline, appends the run.
  - writes go through `tempfile.mkstemp` in the sidecar's directory, then
    `os.replace`, under `fcntl.flock` on `<sidecar>.lock` (never the
    directory's `t09.lock` that `daily.py` holds: flock is per open file
    description, so one file locked twice in one process deadlocks); the temp file is
    removed on any exception. The self-test checks no temp is left.
  - a sidecar that is missing keys, is not JSON or cannot be written: one
    stderr line naming the file and the cause, exit 1, never a traceback.
  - `check` and `stamp` take an optional `now=` for a deterministic test.
- set-valued filters are canonicalised as sorted lists (json.dumps
  refuses a set); any other non-JSON value is a `TypeError`.
- `stamp(path, recipe, coverage)` writes one run's sidecar
  `{"fingerprint", "recipe", "coverage", "at"}`: for a measure captured in
  pieces (a day each) and summed later.
- before `check` or `compare` trusts a sidecar, its stored fingerprint
  must equal `fingerprint(recipe)`; a mismatch is corruption, exit 1, even
  with `--rebaseline`.
- `compare(paths)` reads several stamps: one
  fingerprint passes; mixed fingerprints raise `StampsRefused` naming each
  fingerprint, its files and the changed recipe keys (within 80 columns:
  fingerprints cut to 12 hex, one file per line, long values as indented
  `was:`/`now:` lines); `group_stamps(paths)` never refuses, and
  `compare --rebaseline` uses it and prints each group's fingerprint, files and
  full recipe on stdout, so both recipes are on record.
- CLI: `measure_fingerprint.py compare FILE... [--rebaseline]` exits 0;
  3 on a refusal with the message on stderr; 2 on a usage error (no files
  included); 1 when a stamp is missing, unreadable or not JSON (one stderr
  line naming it, no traceback). The module docstring lists 0/1/2/3.
- `--self-test` runs every case above (sets included) in a temp dir and
  exits 0. `tests/measure_fingerprint.rs` runs it as `python3 -I <script>
  --self-test`, so a sibling `import` fails in cargo as it would in the fleet.
  A second Rust test runs `compare` on two stamps with different recipes in
  a temp dir and asserts exit 3 with the changed key on stderr, and exit 2
  with no arguments. The self-test also asserts the git vectors: empty input
  gives `e69de29bb2d1d6434b8b29ae775ad8c2e48c5391`, `hello\n` gives
  `ce013625030ba8dba906f756967f9e9ca394464a`.
- Callers load it by path (`importlib.util.spec_from_file_location`
  beside the caller), never by `import`: `python3 -I` drops the script's
  directory from `sys.path`, and the fleet runs these with `-I`.

`tools/read-rate.py` gains `--fingerprint FILE` and `--rebaseline`:

- recipe: its own blob; tool version `python3.<minor>` (a refusal on it
  says "interpreter changed"); source
  `claude-code-transcripts`; schema `message.content tool_use/tool_result`;
  filters threshold, MEDIA, DOCS and the plans/tool-results exemptions;
  window `ISO week of the entry timestamp, in its own offset` (what
  `week_of` does today; transcripts write `Z`). `--weeks` only trims the
  print, so it stays out.
- coverage: transcript files seen, files unreadable, weeks printed.
- the docstring usage line gains both flags; its exit codes gain `3 fingerprint refused`.
- a refusal prints the message on stderr and exits 3 before printing the
  table. Without `--fingerprint` nothing changes; `--rebaseline` without
  `--fingerprint` is a usage error (exit 2).
- on success, one stderr line: `fingerprint: baseline written to FILE`, `fingerprint: run N recorded in FILE`, or `fingerprint: rebaselined (keys …), both recipes kept in FILE`.

t09's daily recount (`~/.local/state/t09-recount/`, outside any repo):

- a copy of `measure_fingerprint.py` pinned beside `daily.py`, by blob.
- `daily.py` builds the recipe of the pinned `count-context.py` (blob,
  interpreter version, source `claude-code-transcripts main sessions`,
  schema `compact_boundary + message.usage`, filters `MIN_ASSISTANT=5`,
  window `UTC day, --since D --until D, counted once, never rewritten`).
- the capture is never refused: a day not captured is lost for good,
  because sweeps delete its transcripts. Each counted day gets a stamp,
  `<day>.fp.json`, beside its `.txt`, written before the `.txt` is renamed
  into place: a kill between the two leaves the day uncounted and retried.
  A stamp that cannot be written never blocks the capture: the error goes
  to `<day>.err` and the `.txt` still lands. Each run also stamps any
  `.txt` that lacks one.
- coverage per day: sessions and calls from the day's output, and `late`
  when it was counted more than one day after the day ended
  (2026-11-14..18 are late, as the ticket says: kept as written, with
  `late_reason: counted 2026-11-19 after sweeps`, over the elapsed rule).
- the seven days already counted get their stamps backfilled from the
  same recipe (the pinned blob and interpreter they ran with) and their
  `.txt`/`.err`, run by `/usr/local/bin/python3 -I` (launchd's interpreter);
  `at` comes from the `counted` time in each `.err` when present; the
  11-14..18 `.err` files have none, so their stamps carry `at: null` and
  `counted_on: 2026-11-19` from t09-recount.md, never a file mtime.
- one `flock` on the directory's `t09.lock` serialises a capture run and
  `--sum`. `--sum` globs both `*.txt` and `*.fp.json` and pairs them by
  day: a stamp with no `.txt` or a `.txt` with no stamp is an incomplete
  day, exit 1 naming each file and that `daily.py` recounts or re-stamps
  it (never `--rebaseline`).
- the refusal sits where days are summed. Today the 14 files are added
  by hand at resolution; `daily.py --sum` now does it: it runs `compare`
  on every `<day>.fp.json`, refuses mixed fingerprints unless
  `--rebaseline`, then prints total compactions, calls and the rate, with
  the late days named.
- the new `daily.py` is built and tested in a scratch copy of the
  directory, then swapped in with `os.replace` away from 02:15 and 13:15
  local; the old one stays beside it as `daily.py.prev` until the next
  scheduled run succeeds.

## Phases

- [x] Phase 1 — the helper and its self-test, routed through relais:
  `tools/measure_fingerprint.py`, `tests/measure_fingerprint.rs`
- [x] Phase 2 — same PR as Phase 1: read-rate writes and checks a
  fingerprint, plus the measuring doc: `tools/read-rate.py` (docstring gains exit 3),
  `docs/measuring.md`, `CHANGELOG.md`, and a fixture-transcript case in
  `tests/measure_fingerprint.rs` (baseline, then `--threshold 8192`: exit 3,
  `threshold` on stderr, empty stdout, sidecar bytes unchanged)
- [x] Phase 3 — t09 by hand, outside any repo: pin the helper and wire
  `daily.py`, backfill the stamps: `~/.local/state/t09-recount/daily.py`,
  `~/.local/state/t09-recount/measure_fingerprint.py`
- [x] Phase 4 — decisions note: the fingerprint file and its refusal in
  `docs/programmes/session-boundaries/t09-recount.md` (Window and
  capture), one small docs PR
- [x] Phase 5 — close blue-factory t19: Resolution, map Handoff
  (`docs/programmes/blue-factory/t19-measure-fingerprint-helper.md`,
  `docs/programmes/blue-factory/map.md`)

### 🧑 Your steps

(none)

## Verification

Observed 2026-11-21. `$F` is a frozen copy of 40 transcripts (live ones keep
growing, so two scans of `~/.claude/projects` never match); t09 checks ran
in scratch copies of `~/.local/state/t09-recount`, then live.

| Check | Input | Expected | Actual |
| --- | --- | --- | --- |
| self-test | `python3 -I tools/measure_fingerprint.py --self-test` | exit 0 | exit 0 |
| Rust test | `cargo test --test measure_fingerprint` (`python3 -I`) | pass | 3 passed |
| unchanged without flag | old and new `read-rate.py --root $F` | same bytes | `cmp` equal |
| read-rate baseline | `--root $F --fingerprint S` | table, file written | exit 0, `baseline written` |
| read-rate same | same command again | table, 2 runs | exit 0, `run 2 recorded` |
| refused | `--fingerprint S --threshold 8192` | exit 3, `threshold: 16384 → 8192`, no table, file unchanged | exit 3, `filters.threshold: 16384 → 8192`, no table, sha unchanged, accept command printed |
| rebaseline | same + `--rebaseline` | table, 1 rebaseline entry with both | exit 0, runs 3, rebaselines 1 |
| compare refused | `compare a.json b.json` (two recipes) | exit 3, changed keys named, ≤ 80 columns | self-test: exit 3, keys named, lines ≤ 80 bar whole file names |
| corrupt sidecar | `--fingerprint bad.json` holding `{` | exit 1, file named, no traceback | exit 1, `is not JSON`, one line |
| t09 backfill + sum | launchd argv, then `--sum`, live | totals equal the hand sum | 7 stamped; 8 in 29,641 (hand sum 8 in 29,641; 14..18: 8 in 15,544), late days named |
| t09 refused | one stamp's script blob changed, fingerprint recomputed, `--sum` | exit 3, `script` named; `--rebaseline` prints totals | exit 3, `script` was/now; with `--rebaseline` exit 0, totals |
| t09 corrupt stamp | recipe edited, fingerprint not, `--sum --rebaseline` | exit 1 | exit 1, `stored fingerprint does not match its recipe` |
| t09 unpaired day | one `.fp.json` removed, `--sum`; capture; `--sum` | exit 1, then repaired | exit 1 naming the day and `daily.py`; `stamped`; exit 0 |
| t09 missing day | one day's `.txt` and stamp removed, `--sum` | exit 1 | exit 1, `not counted (no .txt, no stamp)` |
| t09 never refused | counter edited, one day removed, launchd argv | `.txt` and a stamp with the new fingerprint | day counted and stamped; `--sum` then refuses on `script` |
| t09 stamp failure | the stamp's lock path a directory, one day removed, capture | `.txt` lands, error in `.err`, next run stamps | `.txt` landed, `stamp failed` in `.err`, `--sum` exit 1, next run `stamped`, `--sum` exit 0 |
| t09 live | `launchctl kickstart` of the job after the swap | ran line, exit 0 | `ran 2026-11-21T09:30:01`, last exit code 0 |

## Decision log

- relais ran Phases 1–2 (run-660c47b928e3f-424); its verify
  (run-660c4f65cb1ea-18160) failed on the `quiet` test step, which passed
  30/30 but took 1,207 s on a jammed host. The kept candidate was applied by
  hand and checked here.
- Fixed by hand after it: a refusal named a changed dict only as `filters`
  and cut both values, so the change was invisible; `_changes` now walks
  dicts (`filters.threshold`). Paths in a refusal (sidecar, stamp names) are
  never cut. The Rust test asserts the changed value, not `--threshold` in
  the accept command.
- A second session (0f2c38ac) was started on t19 by the same clear-relay
  prompt and worked in this worktree; the person chose this session. Its
  `accept_command` default from `sys.orig_argv` (keeps `-I`) is kept.
- deliberate: a stamp's `at` is when the stamp was written; when a day was
  counted is `coverage.counted_at` (null for 11-14..18, with `counted_on`
  and `late_reason`). `stamp()` has no way to write a null `at`, and this
  keeps a capture time from ever being invented.
- `daily.py --sum` also refuses a window day with neither file (a failed
  count): scratch showed it summed 6 days silently.
- `daily.py.prev` stays beside the live one until t09 resolves.
- CI on the first push: Windows failed on `import fcntl` (the helper is
  Unix only, as its docstring says); the Rust test is now `#![cfg(unix)]`
  and read-rate reports a missing `fcntl` in one line (exit 1, simulated).
  Ubuntu failed in `migrate::tests::a_failure_is_attempted_once_by_hooks…`
  (`Busy` vs `FailedBefore`), a lock-timing test this change does not touch.

## Implementation review

- implementation-review: approve on 95f34925 (round 1 there), after approve-with-changes on 695172d8 and a delta; fixed: flag args (`check`/`rebaseline`, `compare`/`group_stamps`, an `on_change` policy), `let _` reasons, placeholder names, plan text. 71k + 91k + 26k tokens.
- backend: approve (delta); fixed: lock errno, helper load exit 1, coverage `_plain`, mode + dir fsync, Rust cases; deliberate: a dir-fsync error after the rename reads "cannot be written" (rare, errs safe). 66k + 81k.
- lang:python: approve (delta); fixed: typed outcomes (NamedTuple), `RecipeRefused`/`StampsRefused`, `from err`, `__all__`, "ab". 59k + 70k.
- unix: approve (delta); fixed: BrokenPipe in `compare`, 100755, helper load; deliberate: no `--version`, the version is the blob id a recipe records. 62k + 75k.
- deliberate (backend, python): read-rate's `match` has no wildcard; the three outcomes are the helper's whole set and a fourth would be a helper change reviewed with its caller.

<!-- panel: repos=greenbelay,decisions,blue-factory reviewers=backend,lang:python,lang:rust,tui,unix,codex body-sha=b107e8d201f8 -->
