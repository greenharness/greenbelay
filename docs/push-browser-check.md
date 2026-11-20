# The push browser check

Interface work is done when an agent has looked at the rendered result in a
real browser through the `browser-check` agent. Nothing bound that look to
what was pushed: a change could go out with no check, or with the check of an
older tree. The `push-browser-check` rule binds the two: a push that changes
interface files goes out silently only when a `browser-check` of that very
tree completed, and any later edit makes the check stale.

`push-preview` gates the person's approval of a commit and
`implementation-review` gates an independent read of the code. This rule gates
the agent's own look at the page. It asks nobody a question: the only way
through at push time is the check itself, or the plan's declaration below.

## When it speaks

It fires on a push (not a dry run, not a push to `main` or `master`) when all
of these hold:

- the repository has a user interface (a `dev` script, or
  `git config greenbelay.push-preview.ui true`);
- the pushed branch changes a rendered file, detected exactly as
  [`push-preview`](preview.md) does it (tests, test-tool configs and
  comment-only diffs never count);
- no pass is remembered for the pushed tree;
- the landed, approved plan does not declare `## Browser check` → `none: <reason>`;
- no completed `browser-check` agent is bound to the tree (below).

## What binds a check to a tree

The pushed tree is its content id (`greenbelay tree-sha`, 64 hex, with
`docs/plans/` left out). A check counts only when it is a completed
`browser-check` agent in this session's transcript or in one of its side
transcripts (`<session>/subagents/*.jsonl`, where a fork runs its checks), and

- its prompt carries the line `<<<TREE repo=<repo> sha=<id>>>>` of the pushed
  tree (`greenbelay tree-sha --block -C <repo> <rev>` prints it), and
- its result has a line starting `browser: ` that names `chrome-devtools`,
  `playwright` or `claude-in-chrome`, and a line starting `url: `.

A report that stops on `unreachable: <url>`, or names a browser and no page, is
not a check. The report's other content is not judged. Another session's
transcript never counts, and neither does a check of an older tree: a rebase
onto a moved base can change what renders, so there is no carry-over by patch.

On a pass the hook remembers the tree in
`~/.claude/greenbelay/push-browser-check/by-tree/<repo>/<id>.json`. Only the
hook writes there; a Write, Edit or shell write into that directory is refused
whatever the stance, and the refusal names the `browser-check` agent and the
`greenbelay tree-sha --block` line, not the implementation review's remedy.

## The escape

A UI that cannot be served headless is declared in the plan the branch landed:
a `## Browser check` section whose first non-empty line is `none: <reason>`.
It is read as `push-preview` reads `## Preview` → `evidence:`: the plan must be
approved (its body sha is in `plan-approved/`), not a pointer plan, and added
since the merge base with the default branch. A pass this way is journalled
`declared`, with the plan, the commit, the sha and the reason.

## Stance and gaps

Ships at `advise`, ceiling `deny` (`git config greenbelay.push-browser-check.stance deny`).
Under `deny` a push shape the rule cannot read (`--all`, a detached directory)
is held, so it is not a way around the gate. Where no `browser-check` agent is
installed, the rule says so at every stance, as advice and never a hold
(ADR-0073: a missing reviewer is reported, not hidden), so a machine without
the agent hears that its UI pushes go unchecked; `greenbelay doctor` lists the
gap too.

The advice names the tree and prints its block alone on one line:

```
This push changes the interface and no browser check ran in this session. Launch
`browser-check` on the dev server serving this commit, with this line alone on its own
line in its brief:
<<<TREE repo=app sha=…>>>
then push again. …
```

A push run with `git -C <worktree>` names that worktree's tree.

## Measuring

The backtester never runs `confirm`, so it cannot measure this rule. The journal
carries `tree=<12> check=missing|stale|unreachable|gap|unknown` per advice and
`declared` per escape; the 7-day count of `Seen` lines decides graduation
(`greenbelay graduate push-browser-check --to deny`, gated on the reviewed corpus)
or demotion (`greenbelay demote push-browser-check`).
