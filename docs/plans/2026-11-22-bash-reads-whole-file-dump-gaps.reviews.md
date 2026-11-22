# Bash reads: close the `whole-file-dump` gaps: full reviews


Rounds: round 1 is the whole panel; round 2 is a delta; then re-binds after the high edits, Codex's three findings and rust's `||` fix. All seats were resumed through SendMessage.

- **backend** (opus-5-5, ~35–58k per round, 20–55 s): approve-with-changes, then approve. Accepted: tool_result bytes in Phase 0, the sum order and FIFO/missing tests, the kill criterion defined, check expectations ±30%, the picture label, and `No` only under REFUSABLE and MANY_LINES. Rejected: the tui reason (tui was seated instead), and Phase 2 listing hook.rs (the sentence moved to Phase 1).
- **lang:rust** (opus-5-5, ~39–57k, 15–93 s): rework, then approve. Accepted: (blocking) Phase 2 unreachable until `examine` is widened, per-path told keys, per-dump `cwd_at`, no media exemption, a missing operand skipped, parse `Dump.window`, `told` on every path with no short-circuit, and only the right of `||` leaving the sum (`cat big.rs || true` refuses).
- **tui** (opus-5-5, ~31–43k, 13–32 s): approve-with-changes, then approve. Accepted: reason/remedy rebuilt from the refused files, Phase 2's own window reason, the repeat sentence reworded in Phase 1, tui seated in code review, and a wording test.
- **unix** (opus-5-5, ~38–50k, 15–44 s): approve-with-changes, then approve. Accepted: `||` alternatives (later superseded), O_NONBLOCK + fstat, sed without `-s` across operands, and `-c` windows from stat.
- **codex** (high effort): rework. Applied: Phase 0 attributable-only counts; `||` (`cat aa missing || cat bb` prints both); the whole window measured under a 1 MiB command budget.

**Carried into implementation** (implementer: each is a must-do unless the code review waives it):
1. backend: journal one outcome per command (`deny` / `allowed-after-feedback`), not one per path, so the 7-day count compares with Phase 0.
2. backend: `greenbelay check` has no session flag. Run the repeat check through `greenbelay hook` with a payload carrying a fixed `session_id`.
3. backend: `head -c` / `tail -c` over REFUSABLE still need a newline count for the average-line test. Open the file the guarded way.
4. backend + unix: `tail -n N` draws on the same 1 MiB per-command budget, not 1 MiB per file.
5. backend + unix: the `||` right side "may print but is not charged": it is left out of the refusal sum and still gets the advice.
6. backend: restore the `cat aa missing || cat bb` test: `aa` summed, `missing` skipped, `bb` not charged.
7. tui: windowed advice uses the window reason, never "poured whole… no size cap".
8. tui: past the scan budget, the advice says "at least N KB (scan stopped at 1 MiB)" with no average-line clause.
9. unix: a `?` window (`tail -n +5`, `sed -n '5,$p'`, `head -n5`, an unknown flag) is never measured and only advises. Test `tail -n +5` on a long-line file.
10. lang:rust: O_NONBLOCK needs `libc` (`OpenOptionsExt::custom_flags`), new to greenbelay-guard. Name it in the commit.
11. unix (note): `cat big >&2` is not seen as a dump (dump.rs:98). This gap is outside the plan; add a corpus case for it.

