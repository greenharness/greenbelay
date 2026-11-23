+++
type = "task"
mode = "afk"
status = "open"
blocked_by = []
repos = ["greenbelay"]
+++
# Three sessions on one task agree on who owns it

## Question

Two sessions on the same task sort out ownership by SendMessage ("mine" /
"yours"). From three on, they cannot: the claim lives only in the two
conversations, the board names sessions in a namespace SendMessage cannot
address, and the clear-relay hands a resume to a new session while the
owner is still live. What does greenbelay change so that every session,
however many, sees the same owner of a ticket or worktree and can message
it?

## Evidence (2026-11-22/23, decisions follow-up f107)

Three sessions touched f107 (application-landscape theme onto the greenrack
override API, worktree `application-landscape-wt-f107`, branch
`feat/theme-on-override-api`):

- `880b19a1` (SendMessage name `theme-tokens-per-mode-css`): the owner,
  drove the plan.
- `10d13ea9` (SendMessage name `perso-3b [417dc6]`): its first turn was a
  clear-relay prompt, "resume decisions follow-ups
  f107-application-landscape-theme-onto-override-api" (quoted from its
  transcript; the relay log stores no prompt text). Its first
  `greenbelay board --repo application-landscape` printed, while
  `880b19a1` was live:

  ```
  /Users/fredericrous/Developer/Perso/application-landscape-wt-f107 [feat/theme-on-override-api]
    plan: ThemeProvider token values per mode and per contrast, in CSS (pointer)
    session 880b19a1-624f-4508-869c-b10c0c0ba1fc (seen 53m ago)
  ```

  It found the work staged, asked `theme-tokens-per-mode-css` by name,
  got "mine" and backed off. It made no commit, PR, tag or deploy.
- `5310a670` (SendMessage name unknown): its activity file
  (`~/.claude/greenbelay/sessions/5310a670-….activity`) has
  `application-landscape-wt-f107 feat/theme-on-override-api` at
  1795441868 and 1795446355 (2026-11-23 14:51 and 16:05), then the
  worktree `application-landscape-wt-plan-done` on branch
  `docs/theme-tokens-per-mode-done` at 1795448893 (16:48). That branch is
  app PR #352 (merge commit `dfa61854`). Its clear-relay log
  (`~/.local/state/greenbelay/clear-relay/5310a670-….jsonl`) shows arm,
  clear and submit at 1795450238–1795450251 (17:10) from the `decisions`
  cwd. This proves `5310a670` worked in the f107 worktree and landed
  #352, then cleared and relayed. It does not prove who landed #351: the
  activity file and the relay log carry no push or PR.

Then `theme-tokens-per-mode-css`, signing as `880b19a1`, messaged
`perso-3b`: "I see you finished f107 (app#351, v0.67.0, homelab#1287,
decisions#347, f116). Thanks." `10d13ea9` had done none of it. The credit
went by elimination, to the only other session the sender had talked to.
The `880b19a1` relay log also shows a clear at 1795448660 (16:44) before
that message, so a SendMessage name may outlive the `/clear` of the
session it was talking to.

## Gaps

1. **Two namespaces that do not join.** `greenbelay board`, the activity
   files and the `work-overlap` hook name sessions by UUID (`10d13ea9-…`).
   `ListAgents` / SendMessage use a name and a short ref (`perso-3b
   [417dc6]`, which is not a UUID prefix). A session that sees three UUIDs
   on the board can address none of them except by guessing a name.
2. **A claim is a reply, not a record.** "mine" sits in two transcripts.
   The board shows `in flight` and `seen …`, observations, not ownership; a
   third session never learns of the claim.
3. **The clear-relay does not look for a live owner.** clear-relay 0.2.0
   sent the f107 resume to a fresh session while the board listed
   `880b19a1` in that worktree with its plan, seen 53 min before.
   `hooks/register.ts:243-262` arms the relay, and `:213` submits the
   stored prompt. Neither looks at the board or for an owner
   (`~/.claude/plugins/cache/greenbelay/clear-relay/0.2.0`).
4. **Credit has no source.** Commits and pushes do not carry the session
   that made them, so "who landed #351" has no answer to look up.

## Look at

- The board's in-flight rendering and the `work-overlap` hook: where the
  UUID is printed, and whether the SendMessage name/ref can be recorded
  beside it (the session knows both).
- A claim file under `.git/greenharness/` (per ticket or per worktree):
  written on claim, read by the board and quoted by `work-overlap`
  (`owner: <name> [ref] / <uuid>, claimed …`), released at the boundary,
  stale when the owner is not seen for N hours.
- clear-relay: before a resume prompt, read the board for the ticket's
  worktree or plan; a live owner → the prompt names it ("owned by …,
  message it") instead of handing over the work.
- The journal's push line: record the pushing session's UUID, so a PR maps
  to a session.
- Whether a SendMessage name stays with the process across `/clear`, while
  the UUID changes. If it does, a message to the name reaches a session
  that has none of the history the sender assumes, and the claim record
  needs the UUID as well as the name.

## Answer looks like

A plan (one session) whose acceptance is a replay of the f107 shape with
three sessions in a scratch repo: the third session, started by the
clear-relay, is told the owner's name without asking; the board shows one
owner with name, ref and UUID; a push is attributed to its session in the
journal; and no session is credited for work another landed.
