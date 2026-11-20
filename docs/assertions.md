# The assertions

A rule reads a command before it runs. An assertion checks what a command that
already reported success actually did.

The two halves are not the same job. `pipe-to-tail` can refuse
`git push … | tail -5` because the mistake is visible in the command string.
Nothing in a command string tells you that the push you just ran reported
`Everything up-to-date` about a branch you were not on.

| id | fires on | asks |
|---|---|---|
| `push-landed` | `git push` | is the branch on the remote, at the commit you have? |
| `push-published` | `git push` to a UI repository | did it publish a new commit, and was it an approved preview? (records only; see [preview approval](preview.md)) |
| `land-retry` | `amont land`, successful or failed | did the land push the commit the review and preview judged, or a retry nobody judged? Read from the land's receipt, never the tracking ref |
| `bug-nudge` | a check runner that failed (`PostToolUseFailure`), or printed a failure while exiting 0 | did the agent load `diagnosing-bugs` before it ran more commands? Advise only, once per session |
| `home-fingerprint-drift` | a test, lint or build run (`cargo test`, `pnpm test`, `cargo build`, …, also behind `amont slot run --` or `greenbelay quiet --`) or a binary run out of `target/debug/` or `target/release/`, successful or failed; `--help`, `--version`, `--list` and `--no-run` are exempt | did it leave the watched home and repository config files as they were? Advise; see below |

## Successful calls, with three exceptions

Claude Code sends a failed tool call to `PostToolUseFailure`. Everything the
other assertions see claimed to work, which is the whole point: a command that
returned an error is already in front of the model, and there is nothing
invisible left to point out.

`bug-nudge` is the second. A red check runner is the one failure the model may
not read as a bug to diagnose, so it runs on `PostToolUseFailure` too. It asks
for the `diagnosing-bugs` skill once per session, and never blocks.

`land-retry` is the first exception. An `amont land` retries once when its pre-push
hook moves HEAD, and a land that then ends `unverified` or `not-observed` exits
non-zero after the remote accepted the push, so it also runs on
`PostToolUseFailure`. It compares the sha PreToolUse judged (recorded for every
repository) with the land's receipt (amont 1.54.0): another sha `landed` is said
as an unreviewed push, another sha `unverified` or `not-observed` as a possibly
unreviewed one; a refused, unreachable or interrupted land published nothing.
No record, no receipt or more than one is journalled as `retry-check skipped`.

`home-fingerprint-drift` is the third: a red test run can change a home file
as surely as a green one.

## Home fingerprint

Before a command `home-fingerprint-drift` fires on runs in the foreground,
PreToolUse takes a fingerprint of a few files and keeps it under the
journal's directory, in `previews/home/<tool_use_id>` (private, pruned after
24 hours: an interrupted command gets no PostToolUse). The PostToolUse, or
PostToolUseFailure, of the same call takes it again and names each file that
changed:

```text
greenbelay/home-fingerprint-drift: These files changed while `cargo test` ran:
  /Users/me/repo/.git/config (modified)
An edit you made at the same time shows here too.
```

The watched set is the one relais watches around a run (its `homeprint.rs`;
no code is shared): `~/.gitconfig`, `$XDG_CONFIG_HOME/git/config` (else
`~/.config/git/config`), `~/.netrc` (and `~/_netrc` on Windows),
`~/.claude/settings*.json`, `~/.claude/plugins/installed_plugins.json` and
`known_marketplaces.json`, every file under `~/.ssh` but `known_hosts*`, and
the repository's shared `.git/config` (`git rev-parse --git-common-dir`, so a
linked worktree watches its main repository's). `~` is `HOME`, else
`USERPROFILE`; `~/.claude` is `CLAUDE_CONFIG_DIR` when set. `~/.config` as a
whole is not watched: caches rewrite it all day.

A fingerprint holds hashes, never contents: sha256, size and (on unix) mode
bits of each regular file, a symlink by its target (never followed). FIFOs,
sockets and devices are never opened. A directory counts at most 500 files in
byte order; past that the change list ends with `(not checked: over 500
files)`, shown only beside a real change. A file readable on one side only
prints `(not checked: unreadable)`. Paths under `HOME` print as `~/…` on unix,
others as absolute paths.

It is advise-only and cannot refuse. It prevents nothing and restores
nothing: it is the detection net for the 2026-11-15 test run that rewrote the
real home's git config and Claude plugin files, and nothing failed.

## An assertion cannot refuse

The tool has already run. A `deny` stance therefore speaks exactly like
`advise`, the same way it does at a session opening.

What an assertion *can* do is state a fact:

```
greenbelay/push-landed: `git push` exited 0, but origin/main is at d0765d4f9
while the local branch is at 8970c7ffb. The push did not land. Run the push
again on its own and read its output, then confirm with `git ls-remote`.
```

That is not advice to weigh. It is the remote's answer.

## What it refuses to judge

`push-landed` handles the unambiguous shapes and nothing else. A
`HEAD:refs/heads/other` refspec, a tag push, several refspecs at once,
`--delete`, `--mirror`, `--all`: each needs a different question asked of the
remote, and a confidently wrong accusation costs the channel its credibility —
which is the only thing the channel has.

The same goes for everything it cannot establish: a detached HEAD, a remote
given as a URL, a working directory that has gone, a remote that cannot be
reached without a password. All silence.

## It can never prompt

Hooks run with no controlling terminal, so a credential prompt does not fail —
it hangs, and it hangs the session rather than this process. Every child runs
with `GIT_TERMINAL_PROMPT=0`, `GIT_ASKPASS`/`SSH_ASKPASS` disabled, ssh in
`BatchMode`, and a five-second deadline. A guard installed to make pushing safer
must never be the reason a push becomes impossible.

## Measuring

`examine` is pure, so the backtester replays it like any rule:

```console
$ greenbelay backtest push-landed --since 2026-07-06
push-landed   1007   38.7   routine   observe
```

Read that number correctly. For a rule, a firing is a mistake caught. For an
assertion, a firing is a *question asked* — 1,007 pushes in 29,758 Bash calls,
about one call in twenty-six paying for one `git ls-remote`. It speaks only when
the answer disagrees, which is far rarer.

How often a claim is actually broken is not backtestable at all: `verify`
touches the world, and the world has moved since those commands ran. That number
accumulates forward, in the journal:

```console
$ grep push-landed ~/.claude/greenbelay/journal.log
```

`held` is a push that landed, `broken` one that did not, `unverified` one this
crate declined to judge.
