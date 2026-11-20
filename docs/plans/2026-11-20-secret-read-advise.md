---
status: done
plan-id: p-01M7W2VAMA1WZ1M03ZS9JKKQHD
branch: feat/secret-read-advise
repos: [greenbelay]
adrs: []
---
# Plan: `secret-read` — an advise-only greenbelay rule

## Review panel

👉 **Decide:** none — approve if an advise-only rule (never deny, `max_stance` Advise) for secret reads over Bash, Read and Grep is what you want.
📍 greenbelay · 1 PR · next: worktree, then `secret_read.rs` and its corpus. Panel: backend, rust, tui, unix (opus); codex not run.
**Changed by review:** told-once gate moved into `on_bash`; the journal sees the path operand only; `$HOME`, `<` redirects and per-program operands handled.
**Verdicts:** 4 approve-with-changes; 7 carried (shared `secret_file`, glob expansion, remedy quoting, reason tense, told-mark on suppression, one key across tools, "unconfirmed" wording); words 957 → 1360 (×1.42, over ×1.25: each addition guards a false positive, a false negative or a secret kept in the journal); remove 0, merge 0.

📄 Full reviews: [2026-11-20-secret-read-advise.reviews.md](2026-11-20-secret-read-advise.reviews.md)

## Picture

```
 agent tool call
   ├─ Bash  ──► on_bash ──► rules::evaluate ──► secret_read::examine ─┐
   ├─ Read  ──► on_file ──► secret_read block (once per path/agent) ──┤
   └─ Grep  ──► PreGrep ──► secret_read block ────────────────────────┤
                                                                      ▼
                 secret_path(path)? ── no ──► silent
                          │ yes
                 confirm: file exists? ── no ──► silent
                          │ yes
                 Advise (max_stance = Advise): text injected,
                 command STILL RUNS; journal keeps the path only
```

The picture shows the three tool paths (Bash, Read, Grep) meeting one path matcher, then a silent exit or an advice that never blocks.

## Context

A review of claude-code-harness (session e548666f) found that greenharness has
no guard against an agent reading secrets. `permissions.deny` in
`~/.claude/settings.json` is empty and no greenbelay rule looks at secret
paths; the only protection is the habit of not reading them. The reference
harness refuses `cat|grep|cp` on secret paths. The person wants **advise
only**: the agent is told once, the command still runs, and nothing ever denies.

Consequences of advise-only:
- The rule ships with `default_stance: Advise` and `max_stance: Advise`, so
  `greenbelay graduate` and git config cannot raise it (same as
  `screenshot-in-main-session`, `agent-guidance-written`).
- `permissions.deny` stays empty. The "guard the deny list" idea is dropped.
- Legitimate use of credentials (curl with `~/.netrc` or the mTLS cert, per
  `forgejo-recipes`) must stay silent, since only a *read* is the concern.

Out of scope: items 1 (home fingerprint, already in flight as relais
`feat/home-fingerprint`), 2 (test tampering), 4, 5.

## Design

**Rule** `secret-read`, new file `crates/greenbelay-guard/src/rules/secret_read.rs`.
`prevents`: "A secret's bytes poured into the tool result or copied, where
the transcript, the journal and every later turn keep them".

**Pure matcher** `secret_path(&str) -> Option<&'static str>` (the label names
the kind, e.g. "ssh key"). Matches after `~/`, `$HOME/` and `${HOME}/` expansion, quoted or not, in a new
resolver: `whole_file_dump::target` (`rules/whole_file_dump.rs:92-101`) expands
only `~/`, and `dump.rs:174` drops quoted words, so neither is copied. `~user/`
stays unexpanded. Globs (`~/.ssh/id_*`) are matched against the patterns:
- `~/.ssh/*` except `*.pub`, `known_hosts`, `config`
- `~/.netrc`, `~/.aws/credentials`, `~/.docker/config.json`, `~/.kube/config`, `~/.npmrc`
- `~/.config/mtls/*.key`
- `*.key`, `*.pem` (only when `confirm` finds `PRIVATE KEY` in it, so CA bundles
  stay silent), `.env`, `.envrc`, `.env.*` except `.example|.sample|.template`
- `~/.claude/.credentials*`

`~/.claude/settings*.json` is deliberately not matched: it holds configuration,
not secrets (it belongs to the fingerprint feature). A pure `exempt()` in the
module, in the style of `read_unbounded_large::exempt`
(`rules/read_unbounded_large.rs:91`), with a corpus negative pinning each entry.

**Bash shape** (`Examine::Legacy`, modelled on `rules/dump.rs:84 detect`):
- Readers: `cat head tail sed bat more less nl grep rg ugrep awk xxd od
  strings base64 jq cp`. `cp` fires only when the source is secret.
- Unlike `dump.rs`, a pipe or redirect does **not** exempt the reader:
  `cat ~/.netrc | pbcopy` still reads the secret.
- `<` redirect targets are read paths: `jq . < .env`, `cat < ~/.netrc`.
- Operands depend on the program: the first operand of `grep rg ugrep sed awk jq`
  is a pattern, script or filter, not a path, unless `-e`/`-f` supplied it;
  only `cp`'s source counts. Negatives: `grep .env README`, `sed -i … .env` (a write).
- Quiet forms do not fire: `-q -l -c -L` also inside a cluster (`-qi`), their long
  forms (`--quiet --count --files-with-matches`) and `rg --files`; no bytes are shown.
- Not readers, so silent: `curl`, `wget`, `ssh`, `ssh-add`, `git`, `ls`, `stat`,
  `test -f`. This is the curl exemption — no allowlist needed, the program
  name decides.
- `examine` is pure; `confirm` (pattern of `whole_file_dump::confirm`) checks
  the path exists, and returns `Confirmed::No("not there")` otherwise, so
  a typo is not advised about. `confirm` stays read-only: `greenbelay check` runs
  it too (`src/main.rs:3217`), and `Context` carries no agent id. The told-once
  gate goes in `on_bash` after `final_stance`, modelled on `grep_telling`
  (`src/hook.rs:754-778`): `first_telling` keyed `secret-read <resolved path>
  @<agent>` (not `session_state::record`, which records reads); a path told
  before is noted `unconfirmed` and skipped, because the generic Bash loop has
  no told-once gate for an advise rule (`src/hook.rs:787-790`).

**Read tool and Grep tool**: a block in `on_file` (`src/hook.rs:1393`) after
the `read_unbounded_large` block (`:1519-1565`), and one at the top of `on_grep`,
before its early returns (`:2104-2125`). The Grep block fires only when
`output_mode` is `content` and `path` is a file `secret_path` matches; the
default `files_with_matches` and `count` show no bytes and stay silent. Both use
`stance::resolve`, `note_file`, `decision::phrase`, and the same `first_telling`
memory as Bash, so a path is advised once per agent.

**Advice text** states the harm and the way out: use the secret by reference
(`curl --netrc-file`, `--cert`/`--key`, `$TOK`, `ssh-add`), print only what is
needed, and say that the command was allowed. Reason: "`{path}` is a {label}: its
bytes now sit in the tool result, and the transcript and every later turn keep
them. The call still ran." One remedy per label: netrc/credentials "Pass it by
reference (`curl --netrc-file {path}`, a token in `$TOK`); check it with
`wc -c {path}`."; ssh key "Use it through `ssh-add {path}`; never print it.";
`.env` "List key names only: `cut -d= -f1 {path}`."; `cp` "Point the tool at
`{path}` instead of copying the secret." The rule builds its own journal excerpt
(`{label} {path}`) and never passes the command text (`journal.rs:516` would store
it). `examine` sets `Finding.span` to the path operand alone, because every
`Confirmed::No` ("told before", "not there") is journalled through `note` with
the span (`src/hook.rs:717-719,3009`). A unit test runs `grep ghp_abc ~/.netrc`
twice and asserts no journal line holds `ghp_abc`.

## Steps

1. Worktree from `origin/main` (`worktree-task`). Branch name per `AGENTS.md`.
   Land `docs/plans/2026-11-20-secret-read-advise.md` (+ `.reviews.md`) as the
   first commit.
2. `secret_read.rs`: `RULE`, `secret_path`, `exempt`, `examine`, `confirm`, unit tests.
3. Register: `pub mod secret_read;` in `rules/mod.rs`, `secret_read::RULE` in
   `src/rule_table.rs`, corpus entry in `corpus.rs` (~150-200).
4. Corpus `tests/corpus/secret-read.cases`: at least 12 reviewed cases and 4
   negatives. Positives: `cat ~/.netrc`, `cat ~/.netrc | pbcopy`, `grep token
   ~/.aws/credentials`, `cp ~/.ssh/id_ed25519 /tmp/k`, `head -3 .env`. Negatives:
   `curl -n https://x`, `curl --cert ~/.config/mtls/c.crt --key ~/.config/mtls/c.key`,
   `ls ~/.ssh`, `cat ~/.ssh/id_ed25519.pub`, `cat .env.example`, `grep -q x ~/.netrc`.
   Seed with `greenbelay mine --format cases` over real transcripts, then review.
   Add `Read` lines and a `read_judge` arm (`corpus.rs:740`) with a
   `judge_case` fn like `read_unbounded_large.rs:80`.
5. `src/hook.rs`: the `on_file` Read block and the `on_grep` block, with hook unit
   tests: the same path twice gives one Advise (Bash and Read), and the result
   is never Deny.
6. Docs: row in `docs/rules.md` (`advise (max advise)`); bullet under
   `## Unreleased` in `CHANGELOG.md`. Release is a separate `tag-release` step
   (minor bump to 6.6.0), not part of this PR.
7. `Evidence`: measure `per_1000` with `greenbelay mine`/`backtest` over the
   transcripts and fill `measured` in `crates/greenbelay-guard/src/rules/secret_read.rs`
   with today's date; do not invent a figure.
8. Run `greengate agents-md` (updates `AGENTS.md`), stage explicit paths, push through `amont land`
   (`relais-verified-push`), implementation review at F4b, merge with
   `merge-when-green`.

## Verification

- `cargo test -p greenbelay-guard` (corpus agrees with every case; the
  every-rule-has-`prevents` test in `rule_table.rs:73`), then `cargo test`.
- `greenbelay graduate secret-read` must **refuse** to raise it above advise.
- `greenbelay backtest` over the transcripts: report hits and the false-positive
  rate; read every hit that is not a plain secret read. Pass: at most 1 false
  positive in 20 read hits.
- Live check in a fresh session on the installed build: `cat ~/.netrc` advises
  once and still runs; a second read of the same path is silent; `curl -n` and
  `ls ~/.ssh` are silent; a Read of `~/.ssh/id_ed25519` advises.
- Check the journal entry for that run holds the path and no file content.

## Non-goals

- No deny, no `permissions.deny` entries, no per-path user allowlist.
- No change to `read-unbounded-large` or `whole-file-dump`.
- No release in this PR.

## Code review

- greenbelay: backend, lang:rust, unix, tui

## Risks

- False positives on `.env` in repos that commit sample values: `.env.example`
  exempt; a stray advise is cheap because nothing blocks.
- Worktrees: `relais:feat/home-fingerprint` is in flight; both touch
  `rule_table.rs` and `CHANGELOG.md`, so expect a trivial rebase.
- Opaque shell (variable-built paths) is skipped by `Examine::Legacy`; accepted,
  advise-only. Also not covered, accepted: `xargs cat`, `find -exec cat`,
  `grep -r … ~`, process substitution.
- `whole-file-dump` (deny) may still refuse a secret over 8 KB and point to Read
  (`whole_file_dump.rs:114`); out of scope.

## Implementation review

- implementation-review (opus): round 1 approve-with-changes, delta approve-with-changes; flag-argument and exhaustive-match findings fixed. 54k, 44 s; 53k, 24 s.
- backend (opus): both rounds approve-with-changes; Grep told-mark on a symbol-grep deny and hook-path tests fixed; `jq -e` valued-flag arm carried as `deliberate: the file operand is still caught`.
- lang:rust (opus): both rounds approve-with-changes; enums for kind and access done; `Isolated` env lock and `long_arity` `--` lookup carried as `deliberate: low, test-only or cosmetic`.
- unix (opus): both rounds approve-with-changes; value-taking flags and `cp -t` fixed; `&>` and `>|` redirects carried as `deliberate: advise-only, wording at worst`.
- tui (opus): both rounds approve-with-changes; contents wording and per-kind remedies done; docker and claude remedies, copy-advice backticks and tense carried as `deliberate: wording nits`.

## Observed verification

- `greenbelay check 'cat ~/.netrc'` twice: advises (netrc remedy, `curl --netrc-file`), the told-once gate is in `on_bash`, not `check`. `curl -n`, `ls ~/.ssh`, `cat ~/.ssh/id_ed25519.pub`, `grep -q x ~/.netrc`: no rule fires. `cat "$HOME/.netrc"` and `grep ghp_abc ~/.netrc` advise; the `.env` redirect case advises on a real `.env`.
- `greenbelay graduate secret-read --to deny`: refused, "capped at advise".
- `greenbelay rules` lists `secret-read advise (max advise)`; `per_1000` is 0.0, unmeasured (no backtest over transcripts was run).
- Commit gates (clippy, `cargo test --workspace`) passed on all three code commits.

<!-- panel: repos=greenbelay adds= reviewers=backend,lang:rust,tui,unix body-sha=9986028052a2 -->
