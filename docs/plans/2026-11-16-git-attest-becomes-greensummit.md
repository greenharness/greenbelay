---
canonical: greenharness/greensummit:docs/plans/2026-11-16-git-attest-becomes-greensummit.md
phases: [3]
status: active
branch: feat/greensummit-readers
plan-id: p-01M7JZ490DTGP50C29SRAYEMYB
---
# git-attest becomes greensummit

Pointer: the canonical plan lives in greenharness/greensummit. This
repository carries greenbelay's slice of Phase 3: greenbelay reads the
attestation notes under the one greensummit set (`refs/notes/greensummit`,
header `greensummit-v2`), no longer under `greengate-attest` or
`amont-attest`; its provenance names stay. CI runs the greensummit Action
`@v2`, and `allowed_signers` names the `greensummit` namespace (ADR-0073).

## Code review
- amont-agent: backend, lang:rust (platform, unix, tui, architect left out: one names table, tests and a CI line)

## Next in this slice

After this PR merges: release greenbelay, then install it together with
`brew upgrade greengate` (4.11.0) and the attest-signer image bump in
homelab, in one step.

## Verification (observed before the push)

- Suite: `cargo test --workspace --locked --no-fail-fast` green (724 s);
  `cargo clippy --workspace --all-targets --locked -- -D warnings` clean.
  `shipped::tests::gates_are_read_only_under_greensummit` holds: notes under
  `amont-attest` and `greengate-attest` read as absent, the greensummit ref
  with an old header reads as absent, `greensummit-v2` is read.
- Pilot (the published greensummit 2.0.0 x86_64-apple-darwin asset, sha256
  `9f8f0b95…eb4e24` = its SHA256SUMS line, and the worktree's
  `target/debug/greenbelay`; scratch HOME/XDG/global git config; script in
  `~/.claude/greenbelay/attestations/gs-readers/pilot.sh`, three runs):
  - **Writer and reader agree.** Input: `greensummit sign --gates "cargo-test
    clippy"` on a squash merge carrying a signed provenance record.
    Expected: `shipped` shows the gates. Actual: origin holds
    `refs/notes/greensummit` and `-inputs`, header `greensummit-v2`, and the
    row reads `gates cargo-test clippy · verified under greengate-provenance`.
  - **Old name.** Input: the same note moved under
    `refs/notes/greengate-attest`, on origin and locally. Expected: no gates.
    Actual: the row has no `gates` field, still verified, exit 0.

## Decision log

- 2026-11-17 — Gate stamps keep both name sets (`greengate-gate`, then
  `amont-gate`) in push-preflight, though greengate 4.11.0 reads only
  `greengate-gate`: the plan renames attestation names only, and the
  legacy stamp read goes at the 3a-3 census with the provenance one.
- 2026-11-17 — Kept old names only where they prove they are ignored (the
  `shipped` unit test, docs and CHANGELOG saying so) and in
  `tests/fixtures/block/amont-440.diff`, a recorded diff.
