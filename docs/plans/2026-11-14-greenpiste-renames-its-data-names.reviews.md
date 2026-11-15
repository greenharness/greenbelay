# greenpiste renames its data names — full reviews

Shared brief: /tmp/gg-sweep/panel-3b2-brief.md.

**Round 1** (body 41fe174e792e):
- lang:rust, approve-with-changes (34k / 37 s):
  - Rollback after migration loses the journal (high, applied).
  - Hooks must never block or refuse (high, merged).
  - Lock source (merged).
  - Isolated test environment (applied).
  - Key mapping (merged).
  - `names.rs` already exists (applied).
- backend, approve-with-changes (42k / 77 s):
  - `--add` duplicates values on a re-run, and `--list` lowercases names (high, merged).
  - Rollback data (high, applied).
  - Hooks run before migration (high, merged).
  - `tests/plugin_install.rs` missing from Phase 2 (high, applied).
  - Lock placement (merged).
  - Picture (applied).
  - Crash and `doctor` checks (merged).
- lang:typescript, approve-with-changes (52k / 105 s):
  - The plugin already reads `stateDir`; only its fallback changes, and it gets a no-options test (high, applied).
  - `marketplace.json` is re-placed, and reverse writes it from `names::old` (applied).
  - Done- and reversed-markers, a capped wait, and a hook after reverse moves nothing (high, merged).

**Round 2** (body f92e1b107fe2), approve-with-changes from all three, everything carried:
- lang:rust A: a hook can recreate the old directory before the symlink; merge it first.
- backend A: drop "or listed as lost".
- backend B: the markers sit beside the lock.
- backend C: record the first migrating hook's wall time.
- lang:typescript 4: the fallback follows all of `state_root()`, with env-by-name tests.
- lang:typescript 5: the lock goes in `<config>`.
- lang:typescript 6: rows are rewritten on reverse.

**Codex** (body f92e1b107fe2), approve-with-changes:
- Recovery commands bypass the forward migration (high, applied).
- Rename keys in their real config origins, includes and conditions kept (high, applied).
- The plugin registration keeps `enabled=false` and options, and an absent plugin stays absent (medium, carried).

**Final binds:**
- After Codex: lang:typescript and lang:rust approve. lang:rust notes: let git resolve includes (`--show-origin --show-scope`, plus `git config --file` per include).
- backend found a blocker: hooks lose config while step 2 is unfinished. Fixed: old keys are read until done, and a failed-marker stops hook retries (applied).
- On body 2f09c81bd251: all three approve. Carried: an explicit `migrate` clears the failed-marker and retries; a new key wins over the old one in the fallback.

**Carried into implementation (9):**
1. Merge a recreated old directory before the symlink.
2. Rollback wording: rows are rewritten.
3. Markers and lock in `<config>`, outside the roots that move.
4. Record the first migrating hook's wall time.
5. The plugin fallback follows all of `state_root()`, with env-by-name tests.
6. Rows are rewritten on reverse, and a test proves it. Also from the relais and greengate Phase 1 reviews: `--reverse` leaves no file, directory or symlink at `<config>/greenpiste/plan-review/outcomes.jsonl` or `<config>/greenpiste/provenance`, since relais and greengate read the new path first whenever anything exists there; a test asserts both are absent after reverse.
7. The plugin registration keeps `enabled=false` and options; an absent plugin stays absent; a conflict is diagnosed.
8. An explicit `migrate` clears the failed-marker and retries.
9. The fallback prefers the new key and builds old names from `names::old`.

