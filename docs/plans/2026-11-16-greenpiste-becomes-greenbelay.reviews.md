# greenpiste becomes greenbelay — full reviews

## Full reviews (reference)

**Round 1** (body de9d25786206): platform, tui, unix, lang:python, backend approve-with-changes; lang:rust rework (relais `tooling.rs`/`verify` look up `amont-agent`; data one level too deep). All high/blocking applied; unix #1 (close every session) answered with a machine-local copy instead.
**Round 2** (body a6271be90c7a, contract seats added): every seat approve-with-changes; architect blocking and po high (ADR-0068 `harness.renames` contradicted) → Phase 0; tui high (`mv -T`) and unix high (undo writes through the copy) applied.
**Binding** (bodies 69ccd1234888, 14d6e28246bd): backend high (replace only ADR-0068's rename clause) and tui high (`cp` through a dangling brew link → temp copy + atomic rename) applied; all eight bound 14d6e28: approve backend, architect, lang:python; approve-with-changes tui, unix, po, lang:rust, platform.
**Codex** (body 14d6e28): rework: `install --write` would add a second handler beside each old hook (blocking), the stray merge deleted evidence (high), the undo left readers on greenbelay (high) — all three applied; backend re-bound 3b94ca630036: approve-with-changes.

**Carried into implementation (17):**
1. Switch step 0 must allow a re-run: a state file, one line per finished step; a re-run resumes after the last (rust, tui, backend, python, platform).
2. Each step prints one plain line `[n/7] <what> ... ok` (tui).
3. Rehearsal stubs `brew`, `claude`, `greenbelay` on `PATH`, asserts their calls and step order, killed after each step and re-run (python).
4. Step 3 rewrites grh's `receipt.json` part name, paths and argv and drops the `amont-agent` binary-alias entry; `grh status` clean after (rust).
5. `brew fetch fredericrous/tap/greenbelay` before `brew unpin` (unix).
6. The local `/usr/local/bin/greenpiste` copy is removed only when no `claude` process started before the switch; a dated line in `open_checks_due.md` (unix, po, backend).
7. Undo and step 5 both `rm -f /usr/local/bin/.greenpiste.new` (tui, platform).
8. Phase 4 row check: count after ≥ before, and the last pre-switch row present (po, backend).
9. After release: 7 days of judged Bash calls per day vs greenpiste's 7-day median; roll back below 50% on any day or on one exit-127 hook failure (po).
10. Before the tag: `CARGO_REGISTRY_TOKEN` can publish new crate names (`publish-new`), else a new token (platform).
11. `bump-tap.py`'s `amont-agent` and old-owner alternations named in the allowlist (python).
12. `attest.rs:355-358` `holds-until` comment drops the ADR-0068 census (backend).
13. The new ADR restates ADR-0068's sunset text verbatim under the `harness.renames` key (architect).
14. Vendored decision packs in ~6 consumer repos refresh on their next branch or at the census (architect).
15. Step 5's sed rewrites only the binary path; step 7 handles the plugin id and marketplace after its uninstall (backend).
16. The running clear-relay plugin keeps its captured `stateDir`: check `~/.local/state/greenpiste` after the last pre-switch session exits (codex).
17. Check `jq`: exactly one greenbelay handler per event/matcher, 17 in all (backend).

