# Full reviews: Silence only on the armed handoff line

Plan: [2026-11-24-silence-only-on-armed-line.md](2026-11-24-silence-only-on-armed-line.md).
Every seat ran on opus. Round 1 reviewed body `6c0b050f908f`; round 2 and Codex reviewed `2afacd0e818a`.

## Round 1

- **backend**, approve-with-changes (43k tokens, 81 s). All findings accepted:
  1. An `empty-parked` reason, and the first matching row wins.
  2. The PR records both runners' case counts.
  3. The reason id is journalled, and the new Continue volume is counted
     over 7 days before the merge.
  4. The plugin refresh after the release is named.
  5. `make check` gets an observable check.
- **lang:rust**, approve-with-changes (39k, 64 s). All findings accepted:
  1. The checks run in a stated order.
  2. The TS run is a local gate.
  3. The trim set is explicit.
  4. A fixture whose `marker` contradicts its `last_line` fails.
- **lang:typescript**, approve-with-changes (43k, 62 s). All findings
  accepted:
  1. [high] The cases run through `answer()`, not through an export
     `register.ts` does not have.
  2. `log` takes `{sessionId, cwd}`, and `LogEntry` gains a `near-miss`
     variant.
  3. The checks run in a stated order.
  4. There are `\r\n` and U+FEFF cases.
  5. The TS run is a local gate.
- **tui**, approve-with-changes (36k, 36 s). All findings accepted:
  1. The toast puts the reason first, names both forms and what to do,
     and lasts `FAILURE_TOAST_MS`.
  2. The log entry carries the `line`.
  3. Lengths are counted in UTF-16 units.
  4. A disabled relay shows nothing.
  5. A test covers the toast.
- **unix**, approve-with-changes (27k, 21 s). All findings accepted:
  1. One explicit trim set on both sides.
  2. The pilot asserts stdout, stderr and the exit status.
  3. The log is written before the toast, and a failed toast is
     swallowed.

## Round 2

- **backend**, approve-with-changes (51k, 38 s). Round-1 findings
  resolved. New: the no-space widening (C1, medium), the journal field
  (C2, low), and an import fallback (moot: the import was probed and
  works).
- **lang:rust**, approve-with-changes (46k, 31 s). Round-1 findings
  resolved. New: steps 3–5 widen `next:` (C1, medium).
- **lang:typescript**, approve (50k, 27 s). Round-1 findings resolved.
  New: an import fallback (low; moot, since `./handoff-cases.ts` loads in
  `claude plugin test`, probed 2026-11-24).
- **tui**, approve (40k, 11 s). Round-1 findings resolved. New: a
  `parked:x` case (C4, low). The toast wraps at 80 columns, which is
  acceptable because the reason comes first.
- **unix**, approve-with-changes (32k, 13 s). Round-1 findings resolved.
  New: cap the log's `line` (C3, low).

## Codex

Codex returned approve-with-changes (exit 0, no files changed), with C5,
C6 and C7 below, all medium.

## Carried into implementation

- **C1** (backend, lang:rust): check 3 requires `; next: ` with exactly one
  space, the same as today's relay regex. The text after it is read
  untrimmed. There are two cases: `next:`x`` with no space gives
  `not-next-or-parked`, and `next:  `x`` with two spaces gives
  `missing-backticks`. Where the plan's table and its list differ, the
  regex form governs, as the Non-goals say.
- **C2** (backend): the reason id goes into the journal row's excerpt
  (`… near-miss:<id>`), which the exporter's parsing leaves intact. A test
  pins it.
- **C3** (unix): the log's `line` is cut to 2,000 UTF-16 units, and the
  overlong case pins the cut.
- **C4** (tui): `parked:x` with no space is a case, giving
  `not-next-or-parked`.
- **C5** (codex): the TS table test separates the four results. `absent`
  and `parked` give no events or toasts. Each row gets fresh state, and a
  `next` row submits exactly its expected prompt after the countdown.
- **C6** (codex): the two armed corpus copies set `payload.marker: true`,
  so that they pass the consistency check.
- **C7** (codex): the size-kind advise note gains the diagnostic sentence,
  because it has no "ended without that line" sentence to replace. The
  command-kind and size-kind advice are tested separately.
