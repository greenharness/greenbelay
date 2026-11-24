+++
type = "task"
mode = "afk"
status = "open"
blocked_by = []
repos = ["greenbelay"]
due = "2026-12-02"
+++
# Do owner lookups hold up after a week live

## Question

Seven days after greenbelay 6.13.0 (owner records, the resume check, push
attribution), do the `work-owner` journal lines show the registry read
holding, or should the rule go back to `observe` with auto-claims off?

## Look at

- The journal: `work-owner` lines by outcome (`live`, `cleared`, `gone`,
  `stale`, `unknown`, `own`, `none`, `taken-over`, `continued`, `disagree`,
  `pushed`, `lock-busy`, `error`).
- `~/.claude/sessions/*.json` against the `live` lines: a `live` for a pid
  with no registry file is a defect.
- Resume prompts that matched a record against those that stayed silent.

## Answer looks like

Counts per outcome over the week. Roll back (stance `observe`,
`greenbelay.work-owner.auto-claim false`) if any `live` was reported for a pid
with no registry file, if `unknown` exceeds 10% of lookups (a drifted
registry format), or if a steal followed a WARNING. Otherwise keep `advise`
and close.
