# Full reviews: greenpiste tells Claude Code's registry what moved

## Full reviews (reference)

Shared brief: /tmp/gg-sweep/panel-501-brief.md.

**Round 1** (body b11857aecf90), all approve-with-changes:
- tui (38k / 53 s): doctor detail joined by `\x0a` (high, applied); install preview and stderr (applied); shell-quoting (applied); Unknown's next step (applied).
- unix (34k / 51 s): planted leak reads `ok` (high, applied via backend's patch plus `home/`); `--features` breaks greenpiste-process (high, applied via backend); stderr and quoting (duplicates of tui, rejected); a missing file is missing pieces (applied); unknown target exits 2 (applied).
- lang:rust (45k / 73 s): moving the reader breaks agents.rs (blocking, applied); `Unreadable(why)` (high, applied); one public `registry_state` (high, applied); canonical paths (applied); grh pin 4.0.1→5.0.1 (rejected: stale, grh 0.7.0 already pins 5.0.0); `.plugins` key wording (applied).
- backend (44k / 87 s): leak hunt cannot see a canary write + features per package (high, applied); why a stale registry fails (applied); canonical paths (merged with lang:rust); fixtures shaped like the real file (applied).

**Round 2** (body e3fed42322a3): tui approve (28k / 24 s); backend, lang:rust and unix approve-with-changes (35k, 38k, 33k). Applied: the `Unreadable` match at agents.rs:200 (lang:rust, high).

**Codex** (body 167ee67a0d3a): rework. Applied: previews and `--project`/`--local` installs judged against the side they will write (high); migrate's diagnostics after settlement, so an already-migrated machine hears of its registry (high). Carried: `find -printf` is not on macOS BSD find, and a failed snapshot must never read `ok` (medium).

**Final bind** (body 6c5b0946fbd4): backend approve-with-changes (33k / 20 s), no blocker.

**Carried into implementation (9):**
1. `migrate` and `install` print the could-not-read text, with no commands, for `Unknown` (backend r2).
2. Two clean leak-hunt runs leave each canary listing unchanged (backend r2).
3. `install` prints no registry lines when `Env::from_process()` is `None`, with a test (lang:rust r2).
4. An unreadable `settings.json` is `Unknown` (lang:rust r2).
5. The canary listing is portable (BSD and GNU, no `find -printf`) and a failed snapshot makes the target `LEAK` (Codex, backend final).
6. Every leak-hunt argument is checked before the first target runs (unix r2).
7. Verification and Goal wording: the classifier's two entry points, and migrate's output rather than its note (backend final).
8. The doctor JSON keeps `detail` as one newline-joined string (tui r2 note).
9. Fast-forward grh's live checkout before Phase 3 (Codex: it is at 0.6.2).
