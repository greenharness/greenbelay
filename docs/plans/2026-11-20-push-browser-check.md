---
status: active
plan-id: p-01M7WMBDS1NQSP24B9G7HPPGAQ
branch: feat/push-browser-check
repos: [greenbelay, greenharness-skills]
adrs: []
---
# Plan: `push-browser-check`: a UI push carries a browser check of its own tree

## Review panel
📄 Full reviews: [2026-11-20-push-browser-check.reviews.md](2026-11-20-push-browser-check.reviews.md)
👉 **Decide:** none — approve if a browser check bound to the pushed tree, with no person answering at push time, is the right bar.
📍 greenbelay, greenharness-skills · 2 PRs · 1 dated check (day 7: graduate or demote) · next: 30-day baseline count, then the rule. Panel: backend, lang:rust, tui, unix, architect, po (opus).
**Changed by review:** no Overrule question — a plan-declared `## Browser check` escape instead; day-7 graduation/demotion run by the agent; fork side transcripts count.
**Verdicts:** 3 approve, 3 approve-with-changes; 16 carried; words 1116 → 1740 body (×1.56: your two asks, the high fixes); remove 1, merge 0. codex: not run.

## Picture

```
 git push --> UI files changed? --no--> push goes
                   |
                  yes
                   v
   browser-check bound to the pushed tree's content id?
         |                          |
        yes                         no (none, or an older tree)
         v                          v
   pass remembered,           advise (later deny):
   push goes                  "run browser-check on this tree"
```

A push that changes interface files used to go out with no browser check at
all; after this plan it goes silently only when a browser-check of that very
tree completed, and any later edit makes the check stale.

## Context

The fleet rule says web/UI work is done when an agent has looked at the rendered result in a real browser through the `browser-check` agent. This plan applies the "evidence must be newer than the code" idea from HarnessOfHarness: there, a playtest becomes stale as soon as source changes after it.

Today nothing enforces the rule:
- `browser-check` writes screenshots under `~/.claude/greenbelay/attestations/<sha>/`, but that `<sha>` is whatever short sha the caller passed in.
- No greenbelay code reads those files (only `attestations.rs` sweeps them).
- A UI change can be pushed without any browser check, or with a check of an older tree.

`push-preview` gates the **person's** approval of a commit. `implementation-review` gates an independent **code** read bound to the pushed tree. Nothing binds the **agent's own browser check** to the tree being pushed.

**Outcome:** greenbelay rule `push-browser-check`.
- It advises, and can later deny, a push whose branch changes interface files when this session has not completed a `browser-check` agent bound to the pushed tree's content id.
- Any edit after the check changes the content id, so the evidence is stale by construction.

## Design (reuse, don't rebuild)

**Trigger.** Reuse `push-preview`'s detection unchanged:
- The repository has a UI (`crate::preview`: a `dev` script or `git config greenbelay.push-preview.ui true`).
- The pushed branch changes a rendered file (`preview::Needs` and its test, e2e and comment-only exclusions).
- `push_target::find`/`resolve_for` decides what is pushed. Dry runs, deletes and pushes to main or master are skipped, exactly as in `implementation_review_rule.rs:56`.

**Binding.**
- The pushed tree is identified by `crate::content_id::content_id(repo, pushed)`, a 64-hex id with `docs/plans/` excluded.
- A check counts only if all of these hold:
  - it is a completed `browser-check` agent in this session's transcript or in one of its side transcripts, `<session>/subagents/*.jsonl` (forks run worktree-task F2's check there; listed by a new `plan_review::side_transcripts(transcript: &Path) -> Vec<PathBuf>`: `transcript.with_extension("").join("subagents")/*.jsonl`, sorted, empty when the directory is absent; `plan_time::side_files` (:565, a walk of every session) calls it for each session directory instead of keeping its own copy), read with `plan_review::completed_agents_resumable(reader, "browser-check")`;
  - its prompt carries `<<<TREE repo=<repo> sha=<id>>>>` (reuse `implementation_review::block`/`parse_blocks`);
  - its result has a line starting `browser: ` naming one of `chrome-devtools | playwright | claude-in-chrome` AND a line starting `url: `. A report that stops on "unreachable" (one line, `unreachable: <url>`, no `browser:` line) or "no browser" is not a check; a test covers a report that names a browser but stops unreachable.
- The browser-check's own `not checked:` content is not judged. Presence plus a bound tree is the bar, matching what the hook can verify mechanically.

**Memory across sessions and rebases.**
- On a pass, `confirm` writes `<journal>/push-browser-check/by-tree/<repo>/<id>.json` through a small by-tree store in the new module (reusing `is_sha256`, `journal::private`, `write_atomic`); the implementation-review store helpers stay untouched.
- No by-patch carry-over: a rebase onto a changed base can change the render, so only the exact tree passes.
- `guards_path`/`store_clause` (`implementation_review.rs:2525,2580`) extend to the new store, so only the hook writes pass files.

**Escape.** No question to a person: the run is unattended, so the only push-time way through is the check itself.
- A UI that cannot be served headless is declared in the landed, approved plan: a `## Browser check` section whose first non-empty line is `none: <reason>`. It is read exactly as push-preview reads `## Preview` `evidence:` (`preview.rs:1937` `evidence_of` and `preview_evidence` :1884, generalised over the (section, prefix) pair: `Preview`/`evidence:` and `Browser check`/`none:`; push-preview's journal detail stays byte-identical, with a test): same merge-base, same `plan-approved/` sha check, same journal detail. The panel saw the reason, so nobody answers at push time.
- An unreadable push shape is journalled and passes; under deny it is held (`unknown`, as `implementation_review.rs:2048`), so `--all` is not a way around the gate.
- When the `browser-check` agent is not installed (`crate::agents::carried(Need::BrowserCheck, …)`, a new `Need`), the rule returns `Judged::Gap`: said as advice, never held, even under deny (ADR-0073).

**Stance.**
- `default_stance: Advise`, `max_stance: Deny`, `Evidence { measured: today, trend: Rare }` until the 7-day journal count (Verification) gives a real figure; the backtester cannot measure this rule. Before building, count the last 30 days' UI pushes whose session ran no `browser-check`; if that count is 0, close this plan as not needed with the count recorded, and build nothing.
- Graduation is a dated check the agent runs, not a person's call: 7 days after the skills install (item 9), read the journal; if the Verification numbers are met, run `greenbelay graduate push-browser-check --to deny` (it is gated on the labelled corpus, `graduate.rs:32-34`: at least 12 reviewed cases, at least 4 expected-negatives, all agreeing; on a refusal stay at advise and record its lines, never `--force`) and record the figures in `Evidence`; if not, stay at advise and record why. Under advise alone an unattended agent can ignore the text, so the check has a date.

**Wording (examine finding):**
> This push changes the interface and {no browser check ran in this session | the last browser check read tree <old12>, not <new12> | the browser check of <new12> reached no page}. Launch `browser-check` with this line in its brief: `<<<TREE repo=<repo> sha=<id>>>>` (the same as `greenbelay tree-sha --block -C <repo path> <pushed rev>`), on the dev server serving this commit, then push again. A UI this repository cannot serve headless is declared in its landed plan (`## Browser check` → `none: <reason>`).

The hook fills in the block, the path and the rev it already computed, so a push run with `git -C` or of another branch gets the right tree.

## Changes

### Repo 1 — greenbelay (one branch, one PR)
1. `crates/greenbelay-process/src/push_browser_check.rs` (new): `RULE`, `examine`, `confirm`, `judge_push`. The module doc follows `push_preview.rs`'s "Where it speaks" header.
2. `crates/greenbelay-process/src/implementation_review.rs`: add `push-browser-check` to `HOOK_ONLY_DIRS` (:2609) so `guards_path`/`store_clause` cover the new store, and update `GUARD_REASON` to name it. Make the refusal name the store it hit, with its own remedy: for `push-browser-check/` "launch `browser-check` with the `greenbelay tree-sha --block` line", never the implementation-review remedy (`GUARD_REMEDY`, :2613; printed at `src/hook.rs:603,1412`).
3. `src/rule_table.rs`: register the rule. `crates/greenbelay-guard/tests/corpus/push-browser-check.cases`: match/nomatch push shapes, at least 12 labelled cases with at least 4 expected-negatives (the `graduate` floor).
4. `docs/push-browser-check.md` (new) plus a line in `docs/stances.md`'s rule list. CHANGELOG entry.
5. `tests/push_browser_check.rs`: end-to-end with the `World` harness from `tests/implementation_review.rs`. Cases:
   - UI push with no check → advise;
   - check bound to the pushed tree → silent;
   - check bound to an older tree, then an edit → advise (the freshness case);
   - check whose report stops "unreachable" → advise;
   - non-UI push → silent;
   - test-only UI change → silent;
   - rebase with an unchanged patch onto a moved base → advise (no carry-over);
   - an agent Write into the pass store → refused;
   - landed approved plan with `## Browser check` → `none: <reason>` → silent, journalled with the reason; the same section in an unapproved or pointer plan → advise;
   - browser-check completed only in a fork's side transcript, push from the main session → silent;
   - browser-check completed in another session's transcript → advise (only this session and its side transcripts count);
   - `git config greenbelay.push-browser-check.stance deny` → deny;
   - browser-check agent not installed, stance deny → advise (gap), never deny.
6. Release greenbelay as a minor version (`Cargo.toml`, `CHANGELOG.md`) through the `tag-release` skill, then install it on this machine.

### Repo 2 — greenharness-skills (one branch, one PR, after the greenbelay release)
7. `parts/workflow/agents/browser-check.md`:
   - The brief may carry a `<<<TREE repo=… sha=…>>>` line.
   - When it does, file evidence under `attestations/<first 12 of sha>/`.
   - The report contract is unchanged: `browser:` stays the first line, then `url:`. An unreachable URL is reported as exactly one line, `unreachable: <url>`, with no `browser:` line.
8. `~/.claude/skills/worktree-task/SKILL.md` source (`parts/workflow/skills/worktree-task`, lines ~178 and ~425): the UI verification step runs `greenbelay tree-sha --block` after the last commit and puts that line in the browser-check brief. If the tree changes after the check, run it again.
9. Release the skills and bump the grh pin per the usual flow. The greenbelay rule names no skill (every Green Harness part is optional). Its remedy text names only `greenbelay tree-sha --block` and the `browser-check` agent type.

## Verification
- `cargo test --test push_browser_check` → each case in item 5 asserts its named outcome (advise/silent/deny/refused); the corpus replay (`tests/corpus.rs`) → every `push-browser-check.cases` line matches as declared; `tests/implementation_review.rs` still passes after the `HOOK_ONLY_DIRS` edit. The repo's full `greengate` checks run before the push.
- The backtester never runs `confirm` (`journal.rs:135`), so it cannot measure this rule. After the skills release is installed (item 9), read the journal's `Seen` counts for `push-browser-check` over 7 days and record that figure in `Evidence`. Success: at least 70% of UI pushes pass silently on a bound check or a declared escape. Roll back (`greenbelay demote push-browser-check`, ungated) if more than 30% of advises carry an `unreachable:` report: the agent could not satisfy the gate. Also count browser-check re-runs per UI push (cost). The agent runs this check on its date and acts on the numbers (Stance).
- Live check after install:
  - in a scratch worktree of a UI repo (e.g. greenrack), commit a `.tsx` edit and `git push` → advise names the missing check;
  - run browser-check with the `--block` line, push again → silent;
  - edit a file, commit, push → advise again (stale tree).
- implementation-review and the declared code-review profiles at F4b, as for any greenbelay change.

## Non-goals
- Judging the content of a browser-check report (console errors, unanswered questions).
- The tree-hash-before/after mutation check (HoH item 2). Runtime-recorded tool receipts (item 3).

## Code review
- greenbelay: backend, lang:rust, unix, tui
- greenharness-skills: backend (the browser-check brief and report become a contract greenbelay parses)

<!-- panel: repos=greenbelay,greenharness-skills adds=contract reviewers=backend,lang:rust,tui,unix,architect,po body-sha=a2b95056f8ac -->
