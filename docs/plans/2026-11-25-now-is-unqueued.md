---
canonical: amont:docs/plans/2026-11-23-admission-counts-holders-stalled-waits.md
phases: [6]
status: active
---
Part of [Admission counts holders; stalled waits defer](https://github.com/fredericrous/amont/blob/main/docs/plans/2026-11-23-admission-counts-holders-stalled-waits.md): the greenbelay slice, Phase 6.

## Code review

- greenbelay: backend, lang:rust (unix, tui, architect left out: one advise rule's argument parsing)

## Behaviour

From greengate 5.0 a stalled host-slot line ends a wait, `--on-timeout run` is refused (exit 2), and `slot run --now` is the one way to run a command unqueued.

- `heavy-build-unqueued` fires on `slot run --now -- <heavy>` (greengate or amont): the command runs outside the queue.
- Its advice for a bare heavy command names `--now` as the deliberate way past a stalled line, after the queued form and exit 75.
- `--on-timeout=run` stays recognised (older greengate and amont still honour it) but the advice never recommends it.
- Corpus: `match` for `--now`, `nomatch` unchanged for the queued forms.

## Verification

- `cargo test -p greenbelay-guard --locked heavy_build_unqueued`, observed: 8 passed, 0 failed.
  - Input `amont slot run --now -- cargo test` and `greengate slot run --now -- cargo test -p foo`; expected: the rule fires; actual: fires (`a_bare_heavy_command_fires`). The corpus holds `match` lines for both forms.
  - Input `greengate slot run --now -- cargo test -p foo`; expected: the remedy starts "`--now` runs it at once, unqueued", names `greengate slot run -- cargo test -p foo` and `greengate slots`, and never `--on-timeout`; actual: asserted and passing.
  - Input `amont slot run --on-timeout=run -- cargo test` (and the two-word form); expected: the remedy says "`--on-timeout=run` runs it unqueued", never "`--now` runs"; actual: passing.
  - Input bare `cargo test`; expected: the remedy names `--now` and a stalled line, never `--on-timeout`; actual: passing.
- Replay of `greenbelay-guard` as a whole (984 tests) passed before the review fixes; the workspace run and clippy ran at the push gate.

## Implementation review

- implementation-review: approve-with-changes → approve; the `--on-timeout` advice and an empty Verification fixed; 2 candidates fine; 50k+44k.
- backend: approve-with-changes → approve; 3 fixed (older-flag advice, stalled line named, typed tool name); 1 low kept as `deliberate: greengate prints the stall reason beside exit 75`; 55k+42k.
- lang:rust: approve-with-changes → approve; `Launch` enum replaced a bool, test added, gate name carried; 46k+42k.
