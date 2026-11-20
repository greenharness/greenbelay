---
status: active
plan-id: p-01M7W0WBH0ABNE7ZWF4JH0S6RN
canonical: relais:docs/plans/2026-11-20-home-fingerprint-warns.md
phases: 2 (greenbelay assertion home-fingerprint-drift)
branch: feat/home-fingerprint
repos: [relais, greenbelay]
adrs: []
---
# Home fingerprint: warn when a test command changes sensitive files (pointer)

The canonical plan is in relais; Phase 1 (relais) merged as relais#512. This
repository carries Phase 2: the advise-only assertion `home-fingerprint-drift`,
which snapshots the watched set before a test/build command and warns after it
when a file changed. It never refuses.

## Code review

- greenbelay: backend, lang:rust, unix, tui

## Verification

- `cargo test --workspace --locked` → 0 failed → 2,255 passed, 0 failed; clippy `--all-targets -D warnings` → exit 0.
- Piloted, release binary: a scratch crate whose test runs `git config --local x.y 1`, PreToolUse → real `cargo test` → PostToolUse → one advice line `…/crate/.git/config (modified)`; the same test without the write → silent; no record left in `previews/home`.
- `tests/home_fingerprint.rs` (real binary, scratch `HOME`/`CLAUDE_CONFIG_DIR`/`XDG_CONFIG_HOME`): the 11-15 rewrite of the XDG git config warns once, on `PostToolUseFailure` too, and a second Post of the same id is silent; a new `~/.ssh/config` is named, `known_hosts` is not; `git status` keeps no snapshot.
- Unit (`home_print`, `home_fingerprint_drift`, 22): modified/added/removed, mode bits, injected read error, 600-file truncation stable, FIFO skipped, symlink not followed, `USERPROFILE` and empty `HOME`, odd-name round trip, id refusal, common git dir equals git's in a main checkout, a subdirectory and a linked worktree; the 34 corpus cases (20 match, 14 nomatch) replay.
- Budget → ≤ 20 ms p95 per hook side → in-process, 100 runs of `git_common_dir` + `snapshot` on the real home (`~/.ssh` 19 entries, load 11–15, `fingerprint_cost_on_this_machine`, ignored test): p50 3.6 ms, p95 9.5 ms, max 25.6 ms. Whole-hook timing of 100 Pre+Post pairs was host noise (the triggering Pre measured faster than the baseline at load 15–266), so it is not the evidence. The plan's `git rev-parse --git-common-dir` alone measured 46 ms p50, hence the on-disk read.
- Firing rate: `greenbelay backtest --rule home-fingerprint-drift --since 2026-11-13` → 1,221 of 32,123 calls, 38.0 per 1,000 (recorded as the evidence).

## Decision log

- The corpus cases are replayed by the assertion's own unit test: `greenbelay corpus check` scores rules only.
- No `extra_paths` here: the opt-in for `~/.config` is relais's `machine.toml`; greenbelay watches the default set.
- Deviation: the shared git dir is read from `.git`/`gitdir:`/`commondir` on disk, not `git rev-parse --git-common-dir` (46 ms p50 alone, over the 20 ms budget); `GIT_DIR` set for the command only is not seen.
- Deviation: outside a repository the `.git/config` root is dropped, not recorded `unreadable`; the diff stays silent for a path unreadable on both sides either way.
- Added by review: a `homeprint/1` version line (a record another binary wrote is not compared), files over 1 MiB fingerprinted by size+mtime, never read.
