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
