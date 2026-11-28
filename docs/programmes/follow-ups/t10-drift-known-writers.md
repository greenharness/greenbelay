+++
type = "task"
mode = "afk"
status = "open"
blocked_by = []
repos = ["greenbelay"]
+++
# Name the known writers in home-fingerprint-drift

## Question

relais f08 (relais `docs/programmes/follow-ups/f08-home-fingerprint-measure.md`)
counted 23 `home-fingerprint-drift` warnings from 2026-11-21 to 2026-11-26:
11 a repository's `.git/config`, 8 `~/.claude/plugins/*.json`, 6
`~/.claude/settings.json`. greenbelay names no cause, so each reads as
unexplained, and the 4 settings.json changes with no writer are buried
among about 19 a known writer explains. How should the advice fold
known-writer changes the way relais f33 does: Claude Code's plugin refresh
for the two plugin JSONs, `[branch]` tracking and greengate keys for
`.git/config`?

## Look at

relais f33 (`crates/relais/src/homeprint.rs`: `plugin_causes`,
`BRANCH_WRITER`). greenbelay keeps hashes, not contents
(`crates/greenbelay-guard/src/home_print.rs`), so a section-level check
needs either the section bytes kept, or a narrower hash per section.
The home-fingerprint plan keeps no code shared between relais and greenbelay.

## Answer looks like

A plan: which writers are named, what the snapshot keeps so it can tell,
and the advice's shape (unexplained first, one count line for known
writers). The check's p95 stays at most 50 ms; f08 measured 29 ms.
