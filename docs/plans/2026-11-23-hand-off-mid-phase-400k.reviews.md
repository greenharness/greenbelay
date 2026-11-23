# Reviews: Hand off mid-phase at about 400k context

Round 1 (opus; all approve-with-changes):
- architect (45k, 109 s): new ADR rather than an in-place edit; the handoff commit is read as a plan-commit boundary; the size path sits after the `Working`/`stop_hook_active` returns; the in-flight items list was unstated. All accepted.
- backend (36k, 75 s): per-ticket Handoff for parallel tickets; fixtures for 7b; the replay bar measured edits; full ADR path; merge follow-ups into the Phase 3 PR. Accepted. Drop the TTL filing: rejected, because the inbox lines are this session's to file.
- lang:rust (44k, 73 s): three early returns; not a Candidate variant; State is hand-parsed, not serde; hook.rs predicate; replay bar; journal stance. Accepted.
- po (29k, 44 s): deny gated on the replay; a spend target and a cost rollback; a hand sample; fixtures for 7b. Accepted.
- tui (37k, 55 s): the programme prompt lacked the marker line; the 30-character toast; a visible line for the person; relay tests. Accepted.
- unix (34k, 59 s): stance resolver order; State persistence; bad `size-tokens` values. Accepted.

Round 2: po approve (34k), tui approve (42k), architect, lang:rust, unix and backend approve-with-changes. Blocking (backend) and high (unix): the stance clamp cancelled `deny`. Applied.

Codex (high effort): approve-with-changes. #1 high: the exporter replay cannot see mid-phase Stops, so the plan now uses a transcript runner. #3 high: journal rotation (about 3 days) outlives none of the 7-day evidence, so size evidence is kept for 14 days. Both applied.

Carried into implementation:
1. `classify` (clear_boundary.rs:1319) does not skip `docs(plan): handoff` today. Phase 3 adds the exclusion and a fixture, and decision 8's "excludes" is to be read as that change (architect A, lang:rust 7, backend).
2. `plan_phases.rs:145` anchors on the newest `docs(plan):` commit, so a handoff commit would flip `Working` to `PlanOnly`. Skip handoff commits there too, with fixtures (codex 2).
3. hook.rs predicate: `spoke_this_turn && (size_fired_this_turn || PlanOnly)` through a `size_turn` uuid in the state file (lang:rust 4).
4. 7b answers at the higher of the size stance and the command stance, so a size kind shipped at `observe` never silences a command boundary (backend r2 #2).
5. Picture: "pending on a clean tree" (backend r2 #3).
6. The rollback unit is per session: $32.0k / 314 = $102 (backend r2 #4).
7. Say whether a global `greenbelay.stance observe` forces the size kind to observe (backend r2 final).
8. The ADR and the skill state that the per-ticket Handoff is the mid-phase exception to the map's section (architect B).
9. The programme-form ticket may be cut off in the 30-character toast: known, and `/stay` remains (tui r2 1).
10. Fixtures for a `size-tokens` value below 200k and one that does not parse, plus where the complaint is visible (po r2, tui r2 2).
11. A test that the toast's 30-character cut still shows the phase (unix r2 B).
12. decisions gets an architect code review (architect C).
13. The replay coverage bar: at least 95% of Stops past 400k have every blocking fact reconstructed. Below that, the default ships at `observe` and the Phase 4 ticket graduates it to `deny` on live rows with the same 10% held-wrongly bar (backend final).
14. Name the size-evidence file and prune it after 14 days (backend final).
15. Define the handoff subject prefix once, so the `classify` skip and the `plan_phases` skip both use it, in one PR (backend final).

