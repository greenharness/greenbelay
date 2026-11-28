# greenbelay: follow-ups found while doing other work

## Destination

Every follow-up a session noticed outside its own task, and every measure a
plan scheduled for later, is either answered or closed as not worth doing.
Reached when no ticket here is open.

## Notes

- Filed from a session's inbox at its boundary (worktree-task Handoff), one
  task ticket per line, and from a plan's dated check (ADR-0077).

## Decisions so far

- [t05](t05-clear-at-boundary-cap-test-flake.md): the cap test's clock was
  the Stop's 2-second deadline; the Stop tests pin
  `GREENBELAY_STOP_DEADLINE_MS` long, and the shipped deadline stays 2 s.
- [t02](t02-three-sessions-one-owner.md): owner records per worktree and
  ticket, read against Claude Code's session registry (an ADR-0039
  exception); the resume prompt is checked, and pushes are credited only on
  publication evidence (greenbelay 6.13.0).
- [t01](t01-push-browser-check-day-7.md): push-browser-check stays at
  advise on day 7 (57% passes, under 70%; 0 of 47 measured advices
  unreachable); day 14 is [t09](t09-push-browser-check-day-14.md), due
  2026-12-05.

## Handoff

2026-11-24, t02 closed: sessions on one task now agree on its owner. The
board, `work-overlap` and `greenbelay owner show` name the owner by name and
UUID with its state; a `resume <repo> <programme> <ticket>` prompt is told
who owns the ticket; `greenbelay who-pushed` names the session of a push.
Released as greenbelay 6.13.0 and installed here.

- Filed: [t06](t06-one-owner-day-7.md), the 7-day check, due 2026-12-02. To
  relais's inbox: `programme claim` calling `greenbelay owner claim
  --ticket` (then greenbelay's interim observer goes), `next --claim`
  consulting `greenbelay board --json`, and the handoff takeover checking
  liveness. To decisions's inbox: a note scoping ADR-0039's exception for
  the host's session store.
- Next: [t03](t03-size-boundary-day-7.md) and [t04](t04-prompt-cache-ttl-check.md),
  due 2026-11-30; t06, due 2026-12-02; [t09](t09-push-browser-check-day-14.md),
  due 2026-12-05.

- Filed from session 43724200's inbox, 2026-11-28:
  [t08](t08-changelog-keep-both-review-test.md), the keep-both CHANGELOG test.

## Not yet specified

- Whether the resume check should also read `implement <plan> in <repo>`
  prompts, and which record a plan maps to.

## Out of scope

- Nothing.
