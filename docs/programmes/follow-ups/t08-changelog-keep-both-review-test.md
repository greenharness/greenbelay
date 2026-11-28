+++
type = "task"
mode = "afk"
status = "closed"
blocked_by = []
repos = ["greenbelay"]
+++
# A rebase keeping both CHANGELOG entries keeps the review: a test, and its misses

## Question

The only test of the CHANGELOG exception (ADR-0059,
`reviewed-by-patch-notes-moved`) is
`a_rebase_that_only_moves_changelog_context_keeps_the_review`
(`tests/implementation_review.rs:1633`), and it moves an intro line only. The
case the exception exists for, a rebase onto a main that added its own entry
next to the branch's entry, has no test. Does the key hold there, and do the
two near cases miss?

## Also

- A rule file changed on main (the repository's rule files differ between the
  two trees) must miss: the pass does not carry over.
- An `## Unreleased` heading put back (the branch removed or renamed it, and
  the rebase restores it) must miss: the added and removed non-blank lines
  differ, so the key differs.

## Look at

- `tests/implementation_review.rs:1633`: the existing test, to copy its setup.
- `crates/greenbelay-process/src/implementation_review.rs`: `PatchMatch::NotesMoved`
  and the key built from the file's added and removed non-blank lines.
- `docs/implementation-review.md`, the paragraph on `reviewed-by-patch-notes-moved`.

## Done when

Three tests exist: main's entry kept beside the branch's entry journals
`reviewed-by-patch-notes-moved` and the push passes; a changed rule file on
main journals a miss; an `## Unreleased` heading put back journals a miss.

## Resolution

The key holds, and both near cases miss. Three tests in
`tests/implementation_review.rs`:
- `a_rebase_keeping_main_s_entry_beside_the_branch_s_entry_keeps_the_review`:
  a rebase that keeps main's entry beside the branch's entry journals
  `reviewed-by-patch-notes-moved`, and the push is silent.
- `a_rule_file_changed_on_main_beside_moved_changelog_context_needs_a_new_review`:
  an `AGENTS.md` change on main beside a context move does not carry the pass.
- `an_unreleased_heading_put_back_needs_a_new_review`: a heading line put
  back into the branch's patch is advised `(stale)`.

No production change.
