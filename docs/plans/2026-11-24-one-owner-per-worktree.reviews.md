# One owner per worktree and ticket: full reviews

## Full reviews (reference)

Round 1 (body c2a71f66), all opus-5-5:
- tui, approve-with-changes (30k, 27 s). Accepted: claim refusal text and
  exit 3, the WARNING note, 78-column lines, the `who-pushed` format, log
  retention, exact live strings.
- backend, approve-with-changes (34k, 52 s). Accepted: the takeover
  ordering (reworked to parent-pid ancestry), the claim race, the push sha
  in the clause cwd, precedence, journalling `Unknown`, the live check.
  Rejected: picture flows.
- architect, approve-with-changes (36k, 59 s). Accepted: no up-dependency
  (interim hook, `v: 1`), `greengate land` / `--detach`, the reclaim race,
  ticket segment validation. Rejected: the decisions-corpus note (other
  repo; filed to its inbox).
- po, approve-with-changes (33k, 49 s). Accepted: the replay through the
  observed relais claim, plus the silent limit; ref dropped on takeover;
  the after-release section; precedence; current-session live check; full
  slug.
- unix, approve-with-changes (34k, 51 s). Accepted: per-command exit codes,
  tab and `O_APPEND` log, `--steal` in usage, same-dir temp with fsync,
  `board | head` in Verification.
- lang:rust, approve-with-changes (50k, 89 s). Accepted: the caller
  identity source, `read_push` + `cwd_at`, JSON + FNV-1a, `create_new` +
  lock, precedence test, board fixture tests.

Round 2 (body f717237b), resumed:
- backend, approve-with-changes (42k, 26 s). Carried: flock (applied via
  architect), an `auto_claim = false` switch, "first activity" wording.
- tui, approve-with-changes (37k, 17 s). Carried: full UUID in the
  example, a rotation-aware no-match message.
- lang:rust, approve-with-changes (57k, 24 s). Carried: one JSON object
  for context + note.
- architect, approve-with-changes (48k, 28 s). Accepted (high, open):
  reclaim race under flock. Carried: `pub(crate)` `read_push` / `examine`.
- unix, approve-with-changes (41k, 19 s). Carried: rotation under the
  lock, no-match to stderr.
- po, approve-with-changes (41k, 26 s). Carried: the two replay orders
  spelled out, a steal-after-WARNING rollback trigger.

Final delta (body 796c4286), the flock edit:
- backend, approve (41k, 7 s).
- architect, approve-with-changes (47k, 13 s). Carried: name the lock
  dependency. Resolved by lang:rust: reuse
  `crates/greenbelay-guard/src/lock.rs`, MSRV 1.85.
- po, approve (38k, 13 s). Carried: the lock file is never removed; local
  filesystem only.
- lang:rust, resolved (56k, 21 s). Carried: reuse `lock.rs`, 2 s timeout.
- unix, approve-with-changes (42k, 15 s). Carried: `link()` instead of
  `create_new`, so no empty record is visible.
- tui, approve-with-changes (37k, 19 s). Carried: `LOCK_NB` retry, the
  loser exits 3 naming the winner, a hook on a busy lock is silent and
  journals `lock-busy`.

Codex (gpt via codex exec, high effort), approve-with-changes. Accepted
(3 high): publication evidence for push credit, launcher identity for a
detached land, an authoritative ticket record with repo-scoped fallback.
Backend re-ran on that body (body 604a7da4), approve-with-changes (47k,
23 s). Carried:
- for a new branch, read `refs/remotes/<remote>/<dst>` after the push, and
  expand every sha to its full form;
- the launcher's identity goes in `<data dir>/launches/<receipt id>.json`,
  greenbelay's own store, never the land tool's (ADR-0039);
- the prompt's repo name resolves through the ticket's `map_dir` or the
  current repository's worktrees, and no match or several give `Unknown`.
