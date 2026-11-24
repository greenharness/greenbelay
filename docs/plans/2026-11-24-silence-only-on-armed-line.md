---
status: active
branch: prog/session-boundaries/t30-silence-only-on-the-armed-line
repos: [greenbelay, decisions]
adrs: [ADR-0077, ADR-0078]
plan-id: p-01M85ZHMF2W63FT3P3W5RF5QS8
---
# Silence only on the armed handoff line

## Review panel

📄 Full reviews: [2026-11-24-silence-only-on-armed-line.reviews.md](2026-11-24-silence-only-on-armed-line.reviews.md)
👉 **Decide:** none. Approve if one shared grammar, with a near-miss Continue, beats a silent unarmed handoff line.
📍 greenbelay, decisions · 4 phases · next: Phase 1, then a held PR merged after t22. Panel: backend, lang:rust, lang:typescript, tui, unix (opus), codex.
**Changed by review:** an ordered check list with an explicit trim set; a `near-miss` log entry with `line`, plus a toast; the test runs through `answer()`.
**Verdicts:** round 2: 2 approve, 3 approve-with-changes; codex approve-with-changes; 7 carried; words 1516 → 2038 (×1.34, mostly fixes to the grammar's ambiguity); remove 0, merge 0.

## Picture

```
 today                              after this plan
 -----                              ---------------
 last line "safe to /clear; next:   last line --> handoff_line
   resume ..." (no backticks)                       |
   |                                  +-------------+------------+
   +--> hook: contains(MARKER)        |             |            |
   |      --> silent                Next/Parked  NearMiss     Absent
   +--> relay: regex fails          hook silent  hook judges  hook judges
          --> nothing, no log       relay arms   Continue     as today
                                                 names reason
                                                 relay logs,
                                                 toasts reason
```

A handoff line used to silence the hook even when the relay would not
arm on it; after this plan both read one grammar, and a near-miss line
gets a Continue and a toast that name what is wrong.

## Goal

`clear-at-boundary` is silent today whenever the turn's text contains
`safe to /clear` anywhere (`StopView::of`, a plain `contains(MARKER)`). The
clear-relay plugin arms only on an exact last line. On 2026-11-19, session
836b5b71 ended with a bare prompt and no backticks. The rule stayed silent
and the relay did not arm, so nobody cleared the session (t12). After this
change the hook and the relay read one grammar. The rule is silent only when
the relay would arm, or when the line parks. A near-miss line gets a Continue
at `deny` that names what is wrong, and the relay logs and toasts the same
reason (f70).

## Non-goals

- The line's grammar keeps its form. t12, t28 and greenbelay#299 read the
  backticked `next:` form, and ADR-0078 reads the `parked:` form.
- No change to the boundary judgement, the size kind's thresholds or the
  stances.
- The exporter cannot rebuild a last line from a journal row. Exported
  fixtures keep their frozen `marker`.
- Nothing is released here. The release is the next greenbelay release
  after t22 is recorded (see Decision log).

## Code review

- greenbelay: backend, lang:rust, lang:typescript, unix, tui
- decisions: none — markdown only

## Behaviour

### One grammar for the handoff line

`greenbelay_guard::rules::clear_at_boundary::handoff_line(text) -> HandoffLine`
splits `text` on `\n` and reads its last line that is not empty after trimming. Both sides trim the same explicit set, space, tab, CR and LF, and nothing else: Rust `trim` also strips U+0085 and JS `trim` also strips U+FEFF, so neither is used.

| Last line | Result |
| --- | --- |
| does not start with `safe to /clear` | `Absent` |
| `safe to /clear; next: `<p>`` with `p` non-blank, no backtick, ≤ 2,000 UTF-16 units | `Next(p)` (armed) |
| `safe to /clear; parked: <text>` with `text` non-blank | `Parked` (armed: the relay submits nothing) |
| `safe to /clear; next: <p>` where the text after `next:` is not wrapped in backticks | `NearMiss(missing-backticks)` |
| `safe to /clear; next:` with nothing, or blank, between the backticks | `NearMiss(empty-prompt)` |
| the prompt holds a backtick | `NearMiss(inner-backtick)` |
| the prompt is over 2,000 UTF-16 units | `NearMiss(overlong)` |
| `safe to /clear; parked:` with nothing, or blank, after it | `NearMiss(empty-parked)` |
| any other line that starts with `safe to /clear` | `NearMiss(not-next-or-parked)` |

The checks run in order, and the first that fails gives the reason:

1. the line starts with `safe to /clear` (else `Absent`);
2. `; parked: ` follows with non-blank text (`Parked`), or `; parked:` with nothing (`empty-parked`);
3. `; next:` follows (else `not-next-or-parked`);
4. the text after `next:`, trimmed, is not empty (else `empty-prompt`);
5. it starts and ends with a backtick and is at least two long (else `missing-backticks`, as for `` next: `abc `` or `` next: `a` then more ``);
6. the prompt between them is not blank (else `empty-prompt`);
7. it holds no backtick (else `inner-backtick`, as for `` `a`b` ``);
8. it is at most 2,000 UTF-16 units (else `overlong`).

The reason ids are stable. Tests pin the ids, not the wording. Length
counts UTF-16 units (`encode_utf16().count()`), because the relay measures
`.length`.

### The rule

- `StopView` keeps `marker: bool`, which now means "armed" (`Next` or
  `Parked`). It also gains `near_miss: Option<NearMiss>`.
- `says_silence` is unchanged and reads `marker`, so it is silent only on an
  armed line.
  - Text that only contains `safe to /clear` mid-answer no longer silences
    the rule. Neither does a near-miss.
- `size_replay.rs`'s marker also uses `handoff_line`, so the backtest agrees
  with the live hook.
- With `near_miss` set, each remedy says so, and replaces the generic one.
  The handoff is already written, so the agent fixes only the line:
  - command kind, `deny`: Continue
    > This turn's last line starts `safe to /clear`, but the clear-relay
    > plugin does not arm on it: \<reason>. If the handoff is written, fix
    > only that line, with nothing after it: "safe to /clear; next:
    > \`\<the exact first prompt>\`", or "safe to /clear; parked: \<the
    > parked line relais printed>".
  - size kind, `deny`: the same text replaces `size_handoff_remedy`.
  - `advise` (both kinds): the note's "This turn ended without that line"
    becomes "This turn ended with a handoff line the clear-relay plugin does
    not arm on: \<reason>".
- The reason texts are:
  - missing-backticks: the prompt after `next:` is not wrapped in
    backticks;
  - empty-prompt: the prompt is empty;
  - inner-backtick: the prompt holds a backtick;
  - overlong: the prompt is N UTF-16 units, over the 2,000 limit;
  - empty-parked: the parked line is empty;
  - not-next-or-parked: `safe to /clear` is followed by neither `; next: `
    nor `; parked: `.
- A continued turn that ends in a near-miss again gets no second Continue.
  - Below the size boundary, `stop_hook_active` stays silent, as today.
  - Past it, the command Continue already spent the size step and set the
    anchor.
  - A test pins both.
- A near-miss Stop's journal row names the reason id, so the hook side has a production count.

### The relay (f70)

- `handoffPrompt` becomes `handoffLine`, which returns `next`, `parked`,
  `near-miss` with a reason, or nothing.
- A near-miss arms nothing.
  - Only an enabled relay logs or shows anything. It writes one log entry
    `{event: 'near-miss', reason, line}`, where `line` is the trimmed last
    line, so it can be pasted. The entry carries `session`, `cwd` and `time`
    like any other. `LogEntry` gains that variant, with `reason` a literal
    union of the Rust ids, and `log` takes `{sessionId, cwd}` instead of a
    `Relay`.
  - Then it shows a toast for `FAILURE_TOAST_MS`, reason first:
    `clear-relay: not armed: <reason>. It arms on: safe to /clear; next: `<prompt>`, or safe to /clear; parked: <line>. Ask the agent to fix the line, or /clear yourself.`
  - The log entry is written before the toast, and a toast that fails is
    swallowed, so the answer stands.
  - A pending countdown is disarmed (`other-ending`) first, as today.
- A valid `parked:` line still logs nothing, and the existing park test
  keeps `events == []`.
- The relay no longer arms on a prompt holding a backtick: today `(.+)` is
  greedy, so `` `a`b` `` armed. Without this change, the relay would clear
  while the hook sent a Continue on the same turn.

### One table pins both

- `plugin/clear-relay/hooks/handoff-cases.ts` exports `HANDOFF_CASES`. It
  is a strict-JSON array between `// handoff-cases:begin` and
  `// handoff-cases:end`.
  - The plugin runner loads only code files, so a `.json` import is
    refused (probed 2026-11-24).
  - The repository runs no TS formatter.
- Each case holds `line`, or `next_repeat: {unit, count}` (the line is built
  as `` safe to /clear; next: `<unit × count>` ``), plus `kind` and, for
  near-misses, `reason`.
- `register.test.ts` imports the table (a sibling `.ts` import is probed first) and runs every case through `answer()`, the engine, as the suite does today. It asserts an `arm` event for `next`, `events == []` for `parked`, and a `near-miss` event with the case's `reason` and `line` otherwise. A
  `greenbelay-guard` unit test reads the same file with `include_str!`,
  parses the block and runs every case through `handoff_line`.
- The cases cover:
  - the bare prompt;
  - an empty prompt, and a blank one;
  - 2,000 and 2,001 ASCII units;
  - 1,000 and 1,001 × `😀` (2 UTF-16 units each): this pins the counting,
    since `chars()` would pass 1,001;
  - an inner backtick, and 2,001 units holding a backtick (`inner-backtick`);
  - an opening backtick only, and text after the closing one (`missing-backticks`);
  - `parked: next due 2026-11-24` and `parked: nothing due`;
  - an empty `parked:`;
  - `safe to /clear` alone;
  - the marker mid-text with another last line;
  - trailing blank lines;
  - surrounding whitespace, `\r\n` line endings;
  - a line ending in U+FEFF and one ending in U+0085 (both `missing-backticks` on both sides).

### The Stop corpus

- `PayloadFixture` gains an optional `last_line`. When it is present, the
  replay derives `marker` and `near_miss` from `handoff_line`, and it fails
  if the frozen `marker` disagrees. When it is
  absent, the frozen `marker` stands and `near_miss` is `None`, so the 20
  reviewed cases and the `size-*` fixtures stay byte-stable.
- Hand-built fixtures under `stop/clear-at-boundary/`, each from the
  reviewed `match` fixture `52e82064-1794880526.json` with a `last_line`
  added:
  - `near-miss-bare-prompt`, `near-miss-empty-prompt`,
    `near-miss-overlong` and `near-miss-inner-backtick` are labelled
    `match`;
  - `armed-next` and `armed-parked` are labelled `nomatch`.
  - The 836b5b71 Stop has no journal row (it went silent before
    journalling), so it cannot be exported.

## Phases

- [x] Phase 1 — the grammar and its shared table:
  `crates/greenbelay-guard/src/rules/clear_at_boundary.rs`,
  `plugin/clear-relay/hooks/handoff-cases.ts`
- [ ] Phase 2 — the rule reads it, and the near-miss remedies:
  `crates/greenbelay-process/src/clear_boundary.rs`,
  `crates/greenbelay-process/src/size_replay.rs`,
  `crates/greenbelay-process/src/clear_boundary_export.rs`,
  `crates/greenbelay-guard/tests/corpus/clear-at-boundary.cases`,
  `crates/greenbelay-guard/tests/corpus/stop/clear-at-boundary/*.json`
- [ ] Phase 3 — the relay classifies, logs and toasts near-misses:
  `plugin/clear-relay/hooks/register.ts`,
  `plugin/clear-relay/hooks/register.test.ts`, `CHANGELOG.md`
- [ ] Phase 4 — the programme records (decisions, at the ticket's close):
  `docs/programmes/session-boundaries/t30-silence-only-on-the-armed-line.md`,
  `docs/programmes/follow-ups/f70-clear-relay-near-miss-signal.md`,
  `docs/programmes/session-boundaries/map.md`

## Decision log

- 2026-11-24: f70 ships in this PR. It reads the same grammar, so splitting
  it would leave the relay and the hook disagreeing for a release.
- 2026-11-24: the relay refuses an inner backtick too, so that both sides
  give one answer per line. The relay runs from Claude Code's plugin cache,
  not the binary, so until the plugin is refreshed after the release an old
  relay still arms `` `a`b` `` while the hook continues. The release notes
  name the refresh.
- 2026-11-24: the PR is pushed today and merged only after t22's Resolution
  lands (due 2026-11-25). greenbelay releases several times a day, so a
  merge to main before then would ship t30 inside t22's window. The ticket's
  "release with t23 Phase 2" is moot: #298-#302 are already in v6.10.0.

## Verification

- Every `HANDOFF_CASES` row passes, both through `cargo test -p
  greenbelay-guard handoff` and through `claude plugin test
  plugin/clear-relay`.
- `cargo test -p greenbelay-process clear_boundary` covers:
  - a near-miss at `deny` returns a Continue whose text names the reason;
  - an armed `next:` and a `parked:` line are silent;
  - the marker only mid-text no longer silences the rule;
  - a continued turn with a second near-miss past the size boundary gets
    no second Continue;
  - the advise note names the reason.
- `greenbelay corpus check` (the corpus test) passes, with the 20 reviewed
  cases, the `size-*` fixtures and the 6 new fixtures.
- Before merge: over the last 7 days of transcripts, count main-chain final
  messages that contain `safe to /clear` but whose last line is neither
  `Next` nor `Parked`. That count bounds the new Continues at `deny`; the PR
  records it.
- Piloted:
  - the built binary's hook on a Stop payload whose `last_message` ends
    with the 836b5b71 shape, at `deny` with a boundary transcript, prints a
    Continue naming `missing-backticks`;
  - the same payload with the backticks added is silent;
  - on both, stderr is empty and the exit status is 0, and the Continue is
    on stdout only.
- `claude plugin validate plugin/clear-relay` passes. The whole suite
  passes through `claude plugin test plugin/clear-relay`. CI runs no TS
  test, so this is a local gate before the push: the PR body records its
  output and both runners' `HANDOFF_CASES` counts, equal and with 0 failures.
- `register.test.ts` asserts the near-miss toast's text and `timeoutMs`, and
  that an `enabled: false` relay logs and shows nothing.
- `make check` exits 0 on the PR head, and the corpus check lists the 6
  new fixtures with their labels (4 `match`, 2 `nomatch`).

<!-- panel: repos=greenbelay,decisions reviewers=backend,lang:rust,lang:typescript,tui,unix,codex body-sha=2afacd0e818a -->
