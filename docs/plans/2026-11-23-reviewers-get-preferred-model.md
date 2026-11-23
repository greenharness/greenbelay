---
canonical: greenharness-skills:docs/plans/2026-11-23-reviewers-get-preferred-model.md
phases: [2]
status: active
---
Part of [Reviewers get their preferred model wherever they are launched](https://github.com/greenharness/skills/blob/main/docs/plans/2026-11-23-reviewers-get-preferred-model.md): the `agent-preferred-model` rule.

## Code review

- greenbelay: backend, lang:rust, unix, architect (tui left out: the rule's message goes to the agent, not a terminal screen)

## Reversibility

The `preferred-model:` frontmatter key is a contract across three
repositories: greenharness-skills and relais write it, greenbelay reads it.
A rename rolls out in three steps: both writers add the new key beside the
old one, greenbelay reads both (new first), and only then is the old key
dropped. While versions differ, a reader that sees no key it knows passes
the launch, so a half-done rename silences the rule rather than blocking.

## Observed (phase 2)

- `cargo test --workspace --locked --no-fail-fast`: all test binaries ok.
  The rule's unit tests cover a fire, an explicit model, the marker (BOM,
  leading whitespace, CRLF), a pinned definition, no preference, quoted
  values and CRLF frontmatter. `tests/hook.rs` covers a deny for `Agent` and
  `Task`, the ways out, the repository before the user, a plugin file never
  shadowing the user's, a body past the read limit, an unreadable definition
  journalled (`no_frontmatter`, `not_a_file`), and `CLAUDE_PROJECT_DIR` over
  `cwd`. `agents.rs` tests cover `bare_agent_file`'s order, names, plugins
  and a dangling link.
- Replay, `python3 -I tools/agent-preferred-model-replay.py` (transcripts of
  the last 14 days, judged against the definitions installed today): 7362
  Agent/Task launches, 520 reviewer launches without a model, 520 fires
  (expected equal), 0 on any other agent type.
- Deliberate: the check-then-open FIFO race stays (a swap needs write access
  to the agents directory; `O_NONBLOCK` would need a `libc` dependency).

## Implementation review

- implementation-review: approve-with-changes ×2 → approve; 7 fixed; 1 low left (metadata error labelled not_a_file); 62k+39k+39k.
- backend: approve-with-changes → approve; 4 fixed incl. the high (8 KiB refusal); 2 lows fixed in round 3; 70k+34k.
- lang:rust: approve-with-changes ×2 → approve; 7 fixed; 1 low left (unreadable-path test); 46k+32k+24k.
- unix: approve-with-changes → approve; 2 fixed, FIFO race deliberate (no libc); 48k+31k.
- architect: approve-with-changes ×2 → approve; 4 fixed; 1 low left (replay script mirrors the rule: name it); 61k+31k+26k.

Next: release, install, `greenbelay install` for the matcher, stance deny on this Mac, live deny check.
