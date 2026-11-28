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

**Corrections at implementation (2026-11-20).** Four things the body above states too loosely, as built:
- The `browser:` line is matched on its first word: the agent writes `browser: playwright   profile: isolated` on one line, so a whole-line compare would have failed every real report.
- A `url:` whose page starts with `chrome-error:` or `about:blank` is a navigation that failed, not a page; such a report is not a check.
- The missing-agent Gap is said at every stance (never held), as the Stance section says; the first build kept it silent under `advise` because five push-preview record tests in `tests/implementation_review.rs` push UI repos with no `browser-check` installed. Those five tests now set this rule to `observe`.
- An unreadable push shape or unlisted files, under `deny`, are held with "push one branch by name"; an unreadable transcript is held with the `tree-sha --block` line alone. The "missing" advice says "no browser check in this session carried this tree's TREE line", since the transcript cannot tell no check from a check whose brief had no block.

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

**Observed before the push (2026-11-20, greenbelay):**
- Baseline, before building: transcripts of the last 30 days (`~/.claude/projects/*/*.jsonl` and their `subagents/`), branch pushes (`git push` / `greengate|amont land`, not to main, not deletes) from a cwd whose repository has a `dev` script → 573 pushes in 73 sessions; 246 pushes in 34 sessions launched no `browser-check` at all. The proxy counts UI repositories, not UI-changing files (fleet sweeps are in it), so the true figure is lower, and far from 0: built.
- `cargo test --locked --features test-hooks --test push_browser_check --test implementation_review --test corpus` → every case of item 5 asserts its outcome, the five push-preview record tests pass with this rule at `observe`, the corpus replays 17 labelled cases (9 `nomatch`, above the 12/4 graduate floor) → ok, 148 s and 243 s on the two final trees; `cargo test -p greenbelay-process --lib` (unit: `side_transcripts` lists only this session's directory; `report_reached_a_page` fixtures with the `profile:` suffix, `chrome-error://`, `about:blank`; push-preview's `evidence:` journal line byte-identical) → ok; `cargo clippy --workspace --all-targets --features test-hooks -- -D warnings` → clean; each commit's greengate pre-commit gate → 26–28 checks passed.
- The relais worker's candidate (run-660b6591545d8-491a, accepted after three admission waits on a jammed host) had two defects the hand pass caught: the whole-line `browser:` compare and the silent gap; recorded with `relais feedback corrected 0.15`.
- Live check (2026-11-21, greenbelay 6.6.0 from Homebrew, this machine, journal `~/.claude/greenbelay/journal.log`): a scratch repository with a `dev` script and a local bare remote, branch `feat/ui` changing `src/App.tsx`, pushed through this session's own Bash tool (push-preview holds the push at `deny` here, so the publish itself never happened; the rule's verdicts are the journal lines):
  - push with no browser-check in the session → `push-browser-check advise advised … tree=463aa61b7596 check=missing`;
  - `browser-check` (haiku, chrome-devtools) on the served page with the TREE block in its brief, report `browser: chrome-devtools` / `url: http://127.0.0.1:8765/`, then push → `unconfirmed bound`, and `push-browser-check/by-tree/app/463aa61b….json` written (mode 600, `how: transcript`);
  - edit `src/App.tsx`, commit, push → `advise advised … tree=c9f592dbabf1 check=stale`.
- Releases: greenbelay 6.6.0 (`v6.6.0` → eb07f717, release workflow `success` on every job, crates.io 6.6.0, tap 6.6.0, installed here 2026-11-21); skills 0.19.0 (`v0.19.0` → 05359a8d, tarball sha256 b2d10b33…); grh 0.14.2 pins skills 0.19.0.
- Day 7 (2026-11-28): figures and verdict in the Outcome (follow-ups t01: stays at advise); the day-14 check is follow-ups t09, due 2026-12-05.

## Decision log
- 2026-11-20 — Gap said at every stance, not only under `deny`, because ADR-0073 reports a missing reviewer rather than hiding it; the five push-preview tests that push UI repositories with no agent set this rule to `observe`.
- 2026-11-20 — No by-patch carry-over, per the panel: a rebase onto a moved base can change the render.
- 2026-11-20 — `HookOnlyStore` as an enum (rust reviewer, low) left as a follow-up: the string-keyed `guard_of` is the two older stores' shape and a third variant touches implementation-review's guard paths in `hook.rs`.
- 2026-11-20 — A test for the `remember-failed` journal path left out (backend, low): a read-only-directory test is platform-dependent; the live check after install watches the journal instead.
- 2026-11-21 — Rebased onto main's `secret-read` rule (#304): CHANGELOG keeps both Unreleased entries, the rule table counts 57; the first PR head had no CI run because GitHub runs no `pull_request` workflow on a conflicting PR, which is how the drift was noticed.
- 2026-11-21 — CI on the rebased head: green on macOS, Ubuntu, msrv, shell-oracle, audit; red on Windows for two tests that put a temp path into a shell command unquoted (backslashes read as escapes, the hook saw another path, "got Silent"). Fixed by quoting, as the implementation-review tests already do; the worktree test's `tree-sha -C <path>` assertion checks the shape on Windows (`canonicalize` spells `\\?\`, `shell_word` quotes a backslashed word).

## Implementation review
- implementation-review: approve on tree 70bcd6ff (round 1 approve-with-changes, 6 blocking code-rule findings fixed: module doc, banner, swallowed pass write, `_` arm, let-discard reasons, names; Delta approve-with-changes, names on added lines fixed; Delta 2 approve). 116k/56k/40k tokens, 118/82/43 s. After the rebase onto #304 (`secret-read`): approve-with-changes on 4f34b72c (low: the rule-table comment's count), approve on 7b8feb6c once the comment read 57 = 46 + 11. 30k/31k, 27/26 s. The four profiles re-read the rebase: approve (tui's two deferred lows unchanged).
- backend: approve (lost pass write journalled as `remember-failed`; `unknown` split per cause; deliberate: no read-only-dir test for `remember-failed`, `session: "-"` as the rule's other entries). 70k/40k/32k, 80/23/39 s.
- lang:rust: approve on a393af32, carried (exhaustive `Standing`, typed `Pass`, `context.cwd`; deliberate: `HookOnlyStore` enum as a follow-up). 66k/41k, 73/41 s.
- unix: approve on a393af32, carried (`shell_word` quotes `~`/`=`). 68k/40k, 83/19 s.
- tui: approve-with-changes (per-case text, `unknown` remedy, gap sentence, block on its own line fixed); deliberate: under `deny` the `unknown` hold for an unreadable transcript prints the placeholder `tree-sha -C <repo> <rev>` rather than the real values, and a content-id failure points at `tree-sha` too; both are `deny`-only error paths, to be tightened at the day-7 graduation step before deny. 63k/40k/32k, 66/27/33 s.

## Outcome
Shipped 2026-11-21: greenbelay 6.6.0 (the rule, #305), skills 0.19.0 (the brief and report contract, skills#44), grh 0.14.2 (the pin). The three live checks passed on the installed binary. Open: the day-7 check, ticket [follow-ups t01](../programmes/follow-ups/t01-push-browser-check-day-7.md), due 2026-11-28. The ticket holds the counting rule (which journal outcomes are passes, which are advices, the ratio) and the order of the answer: ≥70% passes → the two deny-only hold texts first, as a release, then `greenbelay graduate push-browser-check --to deny` (corpus 17/9, above the 12/4 floor; record the figures in `Evidence`); >30% of advices `unreachable` → `greenbelay demote push-browser-check` (the rule then rests at `observe`: journalled, silent); otherwise stay at advise and record why. Where this paragraph and the ticket differ, the ticket wins. Follow-ups: `HookOnlyStore` enum; the `unknown` hold's real values and the content-id failure wording before deny. Day 7 (2026-11-28, t01): stays at advise, with 57% passes (73 of 128, under 70%) and 0 of 47 measured advices unreachable. Most misses are re-pushes of an advised tree before a check; two of the three repeated trees never got one. Day 14 is [follow-ups t09](../programmes/follow-ups/t09-push-browser-check-day-14.md), due 2026-12-05. Observed on the way, outside this plan: on 2026-11-20 23:21 the host carried load averages 80–222 on 6 cores with 24 Claude processes (`uptime`, `ps`), and the greengate slot queue, not the code, set the pace of every commit, land and relais verify of this release day.

## Non-goals
- Judging the content of a browser-check report (console errors, unanswered questions).
- The tree-hash-before/after mutation check (HoH item 2). Runtime-recorded tool receipts (item 3).

## Code review
- greenbelay: backend, lang:rust, unix, tui
- greenharness-skills: backend (the browser-check brief and report become a contract greenbelay parses)

<!-- panel: repos=greenbelay,greenharness-skills adds=contract reviewers=backend,lang:rust,tui,unix,architect,po body-sha=a2b95056f8ac -->
