# greenbelay

A guard that reads a shell command before Claude Code runs it, and can
observe it, advise against it, or refuse it.

It exists because of a class of mistake no git hook can see, since the mistake
is in the command string and never reaches a hook at all:

```sh
git push origin main 2>&1 | tail -5
```

A pipeline's exit status is the status of its **last** command. `tail` succeeds
at tailing an error message, so a rejected push, a push killed by a timeout,
and a push that never left the machine all report success — and the trimming
discards the error text too, so the failure is silent in both channels.

`greenbelay` (formerly `amont-agent`) is a `PreToolUse` hook. It is a single binary, it is opt-in, and
it phones nothing home.

## The shape of it

Three stances, and the middle one is the point:

| stance | effect |
|---|---|
| `observe` | records the firing and says nothing at all |
| `advise` | puts the reason into the model's context; refuses nothing |
| `deny` | refuses the tool call, with the reason and the remedy |

Of the 40 rules, three ship as `deny`, twenty-four as `advise` and thirteen as
`observe`. A rule is promoted only once your own transcripts say it should be
— see [measuring and graduating](measuring.md) — and `greenbelay rules` shows
the stance in force on your machine.

## Start here

```sh
greenbelay install          # prints the settings block, writes nothing
greenbelay install --write  # merges it into ~/.claude/settings.json
greenbelay doctor           # is it installed, runnable, and actually firing?
greenbelay status           # every rule, its stance, and what it has seen
```

## The inbox read contract

At a session start with source `startup` or `clear`, greenbelay's session
notice reads `<git-common-dir>/greenharness/inbox.md` and every
`inbox.*.filing` file beside it (an abandoned take), and moves nothing.
Documentation version token: `inbox-read/1` (lines carry no token).

Each line is `<YYYY-MM-DD> <session8> [due=YYYY-MM-DD] <text>`. An orphan is a
line dated two or more local calendar days before today, or any line of a take
whose modification time is more than 24 hours old. The filer moves the inbox to
`inbox.<id>.filing` and then `touch`es the take, so its modification time is
the time of the move: a take moved without that `touch` keeps the time of the
inbox's last append, and may be read as abandoned too early. A younger take is
counted as lines being filed and not shown. Only orphan and fresh lines count
as unfiled. Other lines, malformed ones included (session
`?`), are fresh. The block follows the `clear-at-boundary` stance: `observe`
journals the counts and prints nothing.

## Relationship to amont

[amont](https://github.com/greenharness/greengate) is a git-hook manager by the
same author. The two are independent: neither needs the other, they share no
code, and `cargo tree` here shows `serde` and `serde_json` and nothing else.

They meet in exactly one place, and it is optional. If amont is installed and
this repository carries its generated `AGENTS.md` block, a session opening on
a stale block is told so — see [the session notice](session-notice.md). With
no `amont` on `PATH`, that check says nothing.

## Relationship to greensummit

[greensummit](https://github.com/greenharness/greensummit) is the CI end of the same
story: amont signs a note at pre-push naming the gates that really ran, and
greensummit verifies it so CI can skip them. It has no connection to this guard
beyond the author and the conviction all three share — trust what was
verified, never what was reported. The masked push at the top of this page is
what that looks like when it fails.
