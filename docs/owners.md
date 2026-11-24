# Owners: who holds the work

Several sessions on one task need to agree on who owns it, by an address
they can message. greenbelay keeps one owner record per worktree and per
programme ticket, and one line per push that left the machine. Every output
is advice, journalled under the `work-owner` rule (stance `advise`); nothing
is refused.

## Who a session is

Claude Code keeps one file per live session, `~/.claude/sessions/<pid>.json`,
with `sessionId`, `name` (the name SendMessage takes), `pid` and `startedAt`.
greenbelay reads it best-effort, and only when `peerProtocol` is `1`: it is
the host's undocumented store, read under a recorded exception to the
decisions corpus's ADR-0039. A `*.key` file beside it holds peer tokens and is
never opened. `GREENBELAY_HOST_SESSIONS_DIR` points the reader elsewhere.

The calling session is the nearest ancestor of the hook (at most four
processes up) that has a registry file. Against an owner record, the
registry answers one state:

| state | meaning |
|---|---|
| `live` | the same process (pid and `startedAt`) still runs that session |
| `cleared` | the same process now runs another session, after a `/clear`, which has none of the work |
| `gone` | no file for that pid, or a file with another `startedAt` (a reused pid) |
| `stale` | not refreshed for 8 hours, even when live |
| `unknown` | the directory is unreadable, the file does not parse, or `peerProtocol` is not `1`; never reported as `gone` |

## The records

Under `<data dir>/owners/`: `worktree/<hash>.json`, keyed by the worktree's
real path, and `ticket/<repo>/<programme>/<ticket>.json`. Each holds `v: 1`,
the identity `{session, pid, started_at, name, ref?}`, `claimed_at` and
`refreshed_at`. A free record is created with `link()`, so no half record is
ever visible; a reclaim, a takeover and a steal hold an `flock` on
`<record>.lock` (never removed) and read the record again under it, so two
claims never both win.

A `gone`, `stale` or `cleared` record is free. A `cleared` record is taken
over by its own process's new session on that session's first write in the
worktree or its first `resume` of the ticket; a takeover drops the old ref.

Who claims:

- **The first write** in a worktree claims it (a file write, or a git command
  the activity hook tracks), unless it is the primary checkout on its default
  branch. Later writes refresh it. `git config --global
  greenbelay.work-owner.auto-claim false` turns this off.
- **An observed `relais programme claim <programme> <ticket> --dir <dir>`**
  that exits 0 writes the ticket record. This is interim: it goes when relais
  calls `greenbelay owner claim --ticket` itself.
- **`greenbelay owner claim`**, below.

## `greenbelay owner`

```text
greenbelay owner claim [--ticket <repo>/<prog>/<id>] [--name <n>] [--ref <r>] [--steal]
greenbelay owner release [--ticket <repo>/<prog>/<id>]
greenbelay owner show [--worktree <p> | --ticket <repo>/<prog>/<id>] [--json]
```

`claim` exits 0 when claimed, 3 when another live session holds it (stderr
names it and the SendMessage to send), 2 on a usage error, 1 otherwise.
`--ref` records the session's short ref (`[417dc6]`), which only the session
knows. `release` exits 0 when released or already free, 3 when another session
holds it. `show` exits 0 when it shows an owner, whatever its state, and 1
when there is no record.

## The resume check

A prompt starting `resume <repo> <programme> <ticket>` (not a driver's
`… drive: …`) is looked up before the session reads it. The ticket record
answers when it exists; without one, the worktree records on
`prog/<programme>/<ticket>` whose own repository is `<repo>` do, and several
sessions among them answer `unknown`.

- Another live session owns it: the context names it, `<name> [ref] /
  <uuid>`, its worktree and when it was seen, and says to message it with
  SendMessage before any work; a `WARNING:` note tells the person.
- This process owned it before its `/clear`: the record is taken over, and the
  context says `you continue <ticket>`.
- Its owner's process has cleared: the context names the old session and the
  new one, and says not to credit it; a `WARNING:` note tells the person.
- Live records that disagree: all are named, and nothing is taken over.
- No owner, or a gone or stale one: silent. No registered caller: `unknown`.

## Pushes

Exit 0 is not publication. A `git push` is credited only for a ref-update line
in its output (`<old>..<new>  <src> -> <dst>`, `* [new branch]`), never for
"Everything up-to-date"; a land only for a receipt whose status is `landed`.
A detached land's launch saves its launcher, and the `--wait` that sees it
landed credits that launcher once, never the waiting session. The line goes to
`<data dir>/pushes.log` (tab-separated: time, session, name, pid, start,
repository, branch, full sha), which rotates at 1 MB.

```text
greenbelay who-pushed <branch|sha> [--repo <r>] [--json]
```

prints each recorded push, newest first: `<ago>  <name>  <uuid8>  <state>
<branch>  <sha7>`; `--json` carries the full session UUID. With none, it says
`no pushes recorded for <arg>` on stderr and exits 1.

## Limits

- Unix only: the calling session is found by walking parent pids, which
  only works on unix. Elsewhere no record is ever made, so there is nothing
  to show.
- The session file is the host's own, undocumented store; a format change
  reads as `unknown`, never as `gone`.
- A ticket is found only through a ticket record or a worktree on its
  `prog/` branch. An owner working on a feature branch with no ticket record
  is not found by the resume check.
- Only `resume <repo> <programme> <ticket>` prompts are checked.
- A squash merge on the default branch is not mapped back to its branch:
  `who-pushed` answers by branch or by pushed sha.
- The ref is whatever a session reported; the registry does not carry it.
