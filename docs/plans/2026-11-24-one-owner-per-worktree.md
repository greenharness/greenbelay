---
status: active
branch: prog/follow-ups/t02-three-sessions-one-owner
repos: [greenbelay]
adrs: [decisions:ADR-0039, decisions:ADR-0075, decisions:ADR-0083]
plan-id: p-01M85JR92DRA0YDT8AT12KST8W
---
# One owner per worktree and ticket

## Review panel

👉 **Decide:** review grew the plan ×1.55 (exit codes, formats, locks, push evidence); approve if that contract detail stays rather than being cut.
📍 greenbelay · 6 phases · 1 release · next: Phase 1. Panel: architect, backend, lang:rust, po, tui, unix (opus-5-5), codex.
**Changed by review:** claims take an flock and re-read, so two cannot win; pushes are credited only on publication evidence, detached lands to their launcher; the caller is found through parent pids.
📄 Full reviews: [2026-11-24-one-owner-per-worktree.reviews.md](2026-11-24-one-owner-per-worktree.reviews.md)
**Verdicts:** 6 approve-with-changes, codex approve-with-changes; 19 carried; words 1905 → 2951 (×1.55); remove 0, merge 0.

## Picture

```
 today                            after this plan
 -----                            ---------------
 relay --resume f107--> C         relay --resume f107--> C
                        |                                |
                        v                          [prompt hook]
                 board: 3 UUIDs                          |
                 (no name, no owner)        owner record: A, cleared since
                        |                                |
                 C guesses a name           C told: A's process now runs A',
                 credit goes to C              no history; who-pushed: B
```

A resumed session used to guess the owner from UUIDs it could not
message. After this plan the prompt hook reads one owner record and tells
it the owner's name, whether that owner is still live, and who pushed.

## Context

Ticket `docs/programmes/follow-ups/t02-three-sessions-one-owner.md`. On
decisions f107, three sessions worked one task. Two of them sorted out
ownership by SendMessage ("mine"). The third was started by the
clear-relay while the owner was still live. Then the owner credited the
wrong session for work a fourth process had landed. The causes:

- the board names sessions by UUID, which SendMessage cannot address;
- a claim is a reply in two transcripts, not a record;
- nothing looks for a live owner before a `resume …` prompt starts work;
- no push carries the session that made it.

Facts found while planning (2026-11-23):

- Claude Code keeps one file per live session, `~/.claude/sessions/<pid>.json`.
  It holds `sessionId`, `name` (the SendMessage name), `pid`, `startedAt`
  and `peerProtocol`. The file goes away when the session exits, and a
  `/clear` rewrites `sessionId` while `pid` and `name` stay. Measured:
  - 15 files and 15 live pids.
  - This process is `perso-3b`, as it was before its `/clear` (then
    `10d13ea9`, now `a8957506`).
  - The f107 owner `theme-tokens-per-mode-css` (`880b19a1`) now runs
    `9a123c8f`.
- The short ref (`[417dc6]`) is not in that file and not derivable from it.
  Only the session knows it, from the ListAgents "This session is …" line.
- `relais programme claim` stores claims keyed by repo, programme and
  ticket, with no worktree. greenbelay sits below relais (ADR-0075), so it
  never reads that store.
- The f107 owner worked on `feat/theme-on-override-api`, not a `prog/`
  branch. An owner key derived from the branch name would have found
  nothing there.

## Goal

Every session sees the same owner for a worktree or a programme ticket,
with an address it can message (name, ref when known, UUID). It also sees
whether that owner is still the session that did the work: live, cleared
since, or gone.

- A session that the clear-relay starts on a ticket another live session
  owns is told the owner's name before it touches anything.
- A push can be traced to the session that made it.

## Non-goals

- relais changes. Three things are filed to relais's inbox instead:
  - `programme claim` calling `greenbelay owner claim --ticket` (greenbelay's published surface; the record carries `v = 1`);
  - `next --claim` consulting `greenbelay board --json`;
  - handoff takeover checking liveness.
- The clear-relay plugin's TypeScript. The check runs in greenbelay's
  `UserPromptSubmit` hook, which sees the relayed prompt in the new session.
- Prompt shapes other than `resume <repo> <programme> <ticket>[: …]`.
  `implement <plan> in <repo>` is a follow-up.
- Mapping a squash-merge commit on `main` back to its branch. That needs
  the forge. `who-pushed` answers by branch or pushed sha.
- Refusing anything. Every new output is advice (`Advise` / context),
  journalled under the new `work-owner` rule at stance `advise`.

## Code review

- greenbelay: backend, lang:rust, unix, tui, architect

## Behaviour

**Session identity** (new `crates/greenbelay-guard/src/host_session.rs`).
`lookup(uuid)` reads the host registry directory, which defaults to
`~/.claude/sessions` and can be overridden by `GREENBELAY_HOST_SESSIONS_DIR`
for tests. It returns `{pid, started_at, name}` for the live file whose
`sessionId` matches.

`liveness(identity)` compares a recorded `{uuid, pid, started_at}` with the
registry and returns one of:

- `Live`: the same pid and `startedAt`, with the same `sessionId`.
- `Cleared { now: uuid, name }`: the same pid and `startedAt`, with a
  different `sessionId`.
- `Gone`: no file for that pid, or a file with another `startedAt` (a
  reused pid), while the directory is readable.
- `Unknown`: the directory is unreadable, a file fails to parse, or
  `peerProtocol` is not `1`. Unknown is never reported as Gone.

The reader never opens a `*.key` file. That file holds peer tokens.

**Owner records** (new `crates/greenbelay-guard/src/owner.rs`, under
`<data dir>/owners/`). There are two kinds, each written atomically
(a temp file in the same directory, fsync, then rename, so SIGINT, SIGTERM or
a full disk leaves the old record or the new one, never half of one):

- `worktree/<fnv1a64(real worktree path), hex>.json`: `v: 1`, `worktree`,
  `repo_identity`, `branch`, and the identity `{session, pid, started_at,
  name, ref?}`, with `claimed_at` and `refreshed_at`. The records are JSON
  through serde_json, so no new dependency.
- `ticket/<repo>/<programme>/<ticket>.json`: the same identity, plus
  `map_dir`. Each segment must match `[A-Za-z0-9._-]+` and not be `.` or
  `..`; anything else is refused.

A free record is taken with `create_new` when no file exists. Reclaiming an existing `Gone`, stale or `Cleared` record, a takeover, and `--steal` all hold `<record>.lock` and re-read the record under it. The lock is an `flock(2)` lock, which the kernel releases when the holder dies. A test races two claims on an empty slot and on a `Gone` record, and exactly one wins each time. Another kills a lock holder and then claims.

Rules for a record:

- Precedence, tested: `Gone` > stale > `Live`/`Unknown`. A `Gone` owner is
  free. Any owner not refreshed for 8 hours is free even when `Live`
  (ADR-0083 work expiry); the board shows it as `stale`. Otherwise a `Live`
  or `Unknown` owner holds it.
- `Unknown` is decided from the owner's own pid file only; a bad file for
  another pid does not make the answer `Unknown`.
- A `Cleared` owner is held by its process's new session only when that
  session takes the record over. It is the successor (same pid and
  `startedAt`), and it takes over on its first `resume` of that ticket or
  its first write in that worktree. A successor that does neither leaves
  the record `Cleared`, which is free for others.

**Who claims:**

- *Auto, a worktree.* The first session to write in a worktree claims it,
  through a file write or the git Bash commands the activity hook already
  tracks, when the worktree is not the primary checkout on its default
  branch and its record is free. Each later activity of the owner
  refreshes the record at most once per 60 s. The successor in the same process (after
  `/clear`) takes the record over on its first activity.
- *Observed, a ticket.* A Bash `relais programme claim <slug> <id> --dir
  <dir>` that exits 0 writes the ticket record for the calling session.
  The PostToolUse hook does this, so relais itself is not touched. This is interim: a `holds-until:` comment (ADR-0020) names the relais inbox item, and the hook is removed when relais calls `greenbelay owner claim --ticket`.
- *Explicit.* The CLI:
  - `greenbelay owner claim [--ticket <repo>/<prog>/<id>] [--name <n>]
    [--ref <r>] [--steal]` claims the current worktree, the ticket, or
    both. It exits 0 when claimed, 3 when another `Live` session holds it
    and the claim is not `--steal`, 2 on a usage error, and 1 on any other
    failure. On exit 3, stderr says what happened, why, and the next
    command: `refused: <name> [ref] holds <worktree>, live. Message it:
    SendMessage to "<name>". To take over anyway: greenbelay owner claim
    --steal`. `--help` documents the codes. Stdout carries only the result.
  - `greenbelay owner release [--ticket …]` releases them. It exits 0 when
    released or already free, 3 when the record is held by another session,
    2 on a usage error.
  - `greenbelay owner show [--worktree <p> | --ticket …] [--json]` prints
    the owner on stdout. It exits 0 when an owner is shown (any state,
    `Unknown` included), 1 when there is no record, and 2 on a usage error.

**Where an owner is shown:**

- `greenbelay board`: each in-flight worktree gets one line, `owner:
  perso-3b [417dc6] / a8957506-…, claimed 2h ago`, followed by the state on
  its own indented line, state word first, every line at most 78 columns
  (the full UUID is never cut), one of:
  - `live, message it`;
  - `cleared since: perso-3b now runs <uuid>, which has none of this work`;
  - `gone`;
  - `unknown: host registry unreadable`.

  The `--json` output gains `owner: {session, name, ref, pid, state,
  now_session?}`.
- `work-overlap`: the peer text carries the owner's name and ref beside
  the UUID. The remedy names SendMessage to that name when the owner is
  live, and says "cleared since, do not credit it" when it is cleared.
- `UserPromptSubmit`: a prompt starting with `resume <repo> <prog>
  <ticket>` (not `… drive:`) is looked up in the ticket record, which
  is authoritative when held. Only without one does it fall back to the
  worktree records on `prog/<prog>/<ticket>` in that repository (the repo
  named in the prompt, by identity, never by branch name alone). Records
  that disagree are all named in the WARNING and nothing is taken over;
  several fallback owners give `Unknown`. Phase 4 tests both cases and a
  same-named branch in another repository. The new
  `Decision::PromptContext` emits `hookEventName: "UserPromptSubmit"`.
  - The owner is another `Live` session: the context reads `<ticket> is
    owned by <name> [ref] / <uuid> (worktree <p>, seen …): message it with
    SendMessage to "<name>" before any work; take it over only when it
    releases it or the person says so.`
  - The owner is this process before its clear (same pid and `startedAt`,
    another UUID): the record is taken over,
    and the context reads `you continue <ticket>, held by this process as
    <old uuid> before /clear`.
  - The owner is `Cleared`, held by another process: the context reads
    `<old uuid> held it; its process <name> has cleared and now runs
    <uuid>, which has none of this history: do not credit it.`
  - In the first and third cases the hook also emits a `UserNote` of at
    most 2 lines starting `WARNING:`, so the person sees the conflict.
  - `Gone` or none: silent.
  - The caller's own process is the nearest ancestor of the hook with a
    registry file: `parent_id()`, then `ps -o ppid=`, at most 4 levels,
    with `startedAt` from that file. This does not depend on the registry
    having rewritten `sessionId` yet after a `/clear`. When no ancestor is
    found, the context is the `Unknown` line, never "owned by". Phase 4
    tests a stale registry (the old `sessionId` still in the caller's file).
  - Each lookup journals `work-owner` with its state.
  - `Unknown`: one line saying ownership is unknown, and to run `greenbelay
    board`.

**Pushes** (new `<data dir>/pushes.log`). Exit 0 alone is not publication: a
`git push` is credited only when its output has a ref-update line for the
destination (`<old>..<new>  <src> -> <dst>`, or `* [new branch]`), and a
land only when its receipt says `landed`. "Everything up-to-date", a
skipped clause or an unclear outcome credits no one. A credited push
appends this line. The push is read
with `push_landed::read_push`, which refuses `:`/`+` refspecs, several
refspecs and `--delete`/`--mirror`. A land is classified by
`land_retry::examine` (`greengate land` / `amont land`), and a `--detach`
run is not recorded until its `--wait`. The launch saves the
launcher's identity beside its receipt, and the completion credits that
identity, never the waiting session's, once per receipt. A receipt with
no saved launcher stays unattributed. Phase 5 tests B launching, B
clearing, and C waiting twice: one push, credited to B's first UUID. The
line:

```
<ts>\t<session uuid>\t<name|->\t<pid>\t<started_at>\t<repo_identity>\t<branch>\t<sha>
```

Fields are separated by tabs. A tab or newline inside a field becomes a
space, and the name is `[A-Za-z0-9._-]+` or `-`. Each line is appended
with one `O_APPEND` write, and the log rotates at 1 MB. The sha is the `<new>` of that ref-update line (for a new branch,
the source sha `git rev-parse` gave in the clause's cwd, `cwd_at`, before
the push), or the land receipt's pushed sha; never the tip read after the
command. Phase 5 tests `git -C <wt> push origin feat/x`, an unchanged
repeat push by another session (credit stays with the first), and a
commit made after the push (the recorded sha does not move). `greenbelay who-pushed <branch|sha> [--repo <r>] [--json]`
prints each push, newest first, one line each: `<ago>  <name>  <uuid8>
<state-word>  <branch>  <sha7>`; `--json` carries the full UUID. With no
match it prints `no pushes recorded for <arg>` and exits 1. Colour is off
under `NO_COLOR` and when stdout is not a TTY (reuse guard/ui.rs). The board's in-flight branch line adds `pushed by <name> /
<uuid8> <ago>`.

## Phases

- [x] Phase 1: session identity and owner store, with unit tests.
  Files: `crates/greenbelay-guard/src/host_session.rs`,
  `crates/greenbelay-guard/src/owner.rs`,
  `crates/greenbelay-guard/src/lib.rs`.
- [x] Phase 2: claims and the `owner` command, plus the `work-overlap`
  text.
  - Auto worktree claim, an observed `relais programme claim`, and
    `greenbelay owner claim|release|show`.
  - The `work-owner` rule.
  - Files: `src/hook.rs`, `src/main.rs`,
    `crates/greenbelay-guard/src/rules/work_overlap.rs`,
    `crates/greenbelay-guard/src/rules/work_owner.rs`,
    `crates/greenbelay-guard/src/rules/mod.rs`,
    `crates/greenbelay-guard/src/activity.rs`.
- [x] Phase 3: the board's owner line and JSON field.
  File: `crates/greenbelay-process/src/board.rs`.
- [x] Phase 4: the resume-prompt owner check, with `Decision::PromptContext`.
  Files: `src/hook.rs`, `crates/greenbelay-guard/src/decision.rs`,
  `crates/greenbelay-guard/src/payload.rs`.
- [x] Phase 5: push attribution, `greenbelay who-pushed`, and the board's
  pushed-by line.
  Files: `src/hook.rs`, `src/main.rs`,
  `crates/greenbelay-guard/src/owner.rs`,
  `crates/greenbelay-process/src/board.rs`.
- [ ] Phase 6: the f107 replay as an integration test, then docs and a
  release.
  - Files: `tests/three_sessions_one_owner.rs`, `README.md`,
    `CHANGELOG.md`, and the ticket
    `docs/programmes/follow-ups/t02-three-sessions-one-owner.md` with its
    `map.md`.
  - The release is greenbelay 6.11.0, cut with `tag-release`.
  - The release installs the binary on this machine, and the new hooks
    are checked live.

## Decision log

- 2026-11-23: the person chose "Both" for the name source. greenbelay
  reads Claude Code's `~/.claude/sessions/*.json` best-effort, gated on
  `peerProtocol = 1`, for the live name and the pid and `startedAt`
  liveness. A session can also self-report its name and ref through
  `greenbelay owner claim`.
  - This is a recorded exception to ADR-0039. The file is the host's
    undocumented store, not a harness tool's, and no published query
    exists.
  - An unreadable or unexpected file means `Unknown`, never `Gone`.
- 2026-11-23: the ticket key is explicit (`repo/programme/ticket`), never
  derived from the branch name, because the f107 owner was on a feature
  branch.
- 2026-11-23: the check runs in the `UserPromptSubmit` hook, not in the
  clear-relay plugin. It covers a hand-typed `resume …` too, and keeps the
  plugin unchanged.
- 2026-11-23 (panel): the caller's process is found through the hook's
  parent pids, not through SessionStart `source=clear`. That event carries
  neither the pid nor the old UUID. A takeover there would also hand
  ownership to a cleared session that never resumes the work, which is
  f107's credit error.
- 2026-11-24 (implementation): deviations from the body above.
  - The release is 6.13.0: 6.11.0 and 6.12.0 shipped on main meanwhile.
  - The push log has its own reader in `owner/pushes.rs`, sharing
    `push_landed`'s refused-flag list, instead of `push_landed::read_push`.
    `read_push` takes `git -C <dir> push` for three operands and refuses it,
    while this plan's own check names that shape; teaching it `-C` would
    point the `push-landed` assertion, whose directory ignores `-C`, at the
    wrong repository.
  - The crediting runs in the guard crate (`owner::pushes::credit_after_bash`),
    not in `src/hook.rs`: the binary cannot reach `pub(crate)` items.
  - The owner CLI and `who-pushed` live in `owner/cli.rs`, exempted from the
    one-stdout-writer test like the other command-line verbs. Done messages
    (`claimed`, `released`, `already free`) go to stderr; stdout carries only
    `show` and `who-pushed` data.
  - Board lines wrap at 78 columns between chunks that never split, so the
    example `owner: … / <uuid>, claimed 2h ago` takes two lines when it
    does not fit; `/` travels with the UUID.
  - A record that does not read holds its slot (exit 3, journal `unknown`);
    only `--steal` replaces it. The plan had left that case open.
  - The owner feature is unix only: the caller walk needs parent pids, so
    off unix it makes no claim. `tests/owner_hooks.rs` and
    `tests/three_sessions_one_owner.rs` are `#![cfg(unix)]`, like
    `tests/board.rs` (Windows CI failed them on PR #328).

## Verification

Each check lists what is driven, then what should be observed.

- Phase 1: unit tests against a sandboxed registry dir with fixture files
  for live, cleared (same pid, new `sessionId`), reused pid, missing, and
  unparseable or `peerProtocol = 2`. The result should be `Live`,
  `Cleared`, `Gone`, `Gone` and `Unknown`, and no `*.key` file opened.
- Phase 2, end to end with a sandboxed HOME:
  - A hook payload of a write by session A in a feature worktree creates
    the record.
  - A write by B there yields `work-overlap` text naming A's name and
    `SendMessage`.
  - `greenbelay owner claim` by B exits 3.
- Phase 3: board unit tests over a fixture registry assert each of the four state lines and the JSON `owner` field; Phase 1's fixture `*.key` is mode 000 so an open fails. `greenbelay board | head -1` exits 0 with nothing on stderr. Then
  `greenbelay board --repo greenbelay` run live on this machine prints an
  `owner:` line for this task's worktree, naming this session's current
  name and UUID (read from its registry file at check time) and the exact
  `live, message it`.
- Phase 4: a `UserPromptSubmit` payload `resume decisions follow-ups f107-application-landscape-theme-onto-override-api`
  is sent while a fixture ticket record is held by a live other session.
  The context should name the owner and SendMessage. When the record's pid
  is the caller's own, it should say takeover.
- Phase 5: a PostToolUse payload for `git push origin feat/x`, exit 0,
  appends one line. `greenbelay who-pushed feat/x` should name that
  session.
- Phase 6: `tests/three_sessions_one_owner.rs` replays f107 in a scratch
  repository with a fake registry.
  1. A owns a feature (non-`prog/`) worktree by an auto claim (its first write) and the ticket only by an observed `relais programme claim … --dir` exit 0, with no `greenbelay owner claim`.
  1b. A second run with no ticket record (worktree record only, on `feat/…`): the test asserts C is silent, and the README states this limit.
  2. A clears: same pid, new UUID A′.
  3. C is started with `resume <repo> follow-ups
     f107-application-landscape-theme-onto-override-api`.
  4. B lands a push from another worktree.
  5. A second order: A′ resumes the ticket first, then C does.

  The test asserts all of these:
  - In order 1, C's prompt context names A's process name and says it
    cleared, with no credit to A′.
  - In order 2, A′ takes the record over, and C is told it is owned by
    A's name, live.
  - After A′'s first write, the board shows exactly one owner, A′, with
    name and UUID; A's ref is dropped on takeover and shown again only after A′ runs `owner claim --ref`.
  - `who-pushed` names B and not C.

## Verification results

Observed on the final tree before the push (input → expected → actual):

- Phase 1: `host_session` and `owner` unit tests → five states, no `*.key`
  opened, one winner per race, a SIGKILLed lock holder freed → all pass.
- Phase 2: `tests/owner_hooks.rs`, `tests/owner_cli.rs` → record created on
  A's write; B's write told `alpha-name` and SendMessage; B's claim exits 3
  → all pass.
- Phase 3: board unit tests (fixture registry, mode-000 `*.key`) → each
  state line, every line ≤ 78 columns, the JSON `owner` field → pass. Live:
  `greenbelay board --repo greenbelay` → this worktree's `owner:` line with
  this session's registry name `single-owner-worktree-ticket` and UUID
  `a8957506-…`, then `live, message it`, nothing on stderr; `board | head -1`
  → exit 0.
- Phase 4: the prompt tests (live other, successor under a stale registry,
  cleared other, fallback, other repository, disagreement, no caller,
  driver prompt) → pass. Live: a resume of this ticket as this session →
  silent, journalled `work-owner advise own`.
- Phase 5: real `git push` output into a scratch remote, `git -C`, an
  unchanged repeat, a detached land waited twice → two lines for B, none
  for C, one for the launcher → pass. Live: `who-pushed` of this branch →
  `no pushes recorded for …`, exit 1.
- Phase 6: `tests/three_sessions_one_owner.rs` → both orders, the no-ticket
  limit, one owner on the board with the ref dropped and re-reported, B
  credited and not C → 3 of 3 pass.
- Workspace: clippy `-D warnings` and `cargo test --workspace` → clean; the
  commit gate's 28 checks → passed.

## Implementation review

- implementation-review approve (r2; r1 approve-with-changes, 3 blocking fixed: results, stderr, discards); 115k+112k, 128+58 s, opus.
- backend, unix, tui approve-with-changes (r2): fixed unreadable-record hold, refusal text, takeover failure, launcher order; carried lows: release of an unreadable or gone-held record, wait race, ps-timeout skip.
- lang:rust approve-with-changes (r2): carried `Continued` flag+option → enum, typed `take_over` error, `Option` names and placeholders, purity split.
- architect approve (r2): peer_owners takes the store, test pid sandboxed; carried: one construction site for the store.
- deliberate: carried items go to a follow-up ticket via the greenbelay inbox; distinct failure exit code and `--` with them.

## After release (7 days)

- Live, right after the install: the session running the check runs
  `greenbelay owner show --json` and the board. Both should name its own
  registry `name` and current `sessionId`, with `"state":"live"` and
  `live, message it`.

- Count `work-owner` journal entries by state, and resume prompts that
  matched a record against those that stayed silent.
- Roll back (rule stance `observe`, auto-claims off) if any `Live` is
  reported for a pid with no registry file, or if `Unknown` exceeds 10% of
  lookups, which would mean the registry format has drifted.

<!-- panel: repos=greenbelay adds=contract,cli reviewers=architect,backend,lang:rust,po,tui,unix,codex body-sha=604a7da4ab8f -->
