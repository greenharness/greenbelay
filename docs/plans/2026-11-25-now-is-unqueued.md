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

- `cargo test -p greenbelay-guard`: the unit cases and the corpus replay pass.
- The rule's message for `greengate slot run --now -- cargo test` is read from a hook run.
