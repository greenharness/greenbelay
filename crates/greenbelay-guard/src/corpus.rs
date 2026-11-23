//! Reviewed judgements, kept as a test.
//!
//! A rule's fire rate says how *often* it speaks. It says nothing about whether
//! it is right, and being right is the whole product: recall is recoverable —
//! a rule that misses something can be widened next week and the backtester
//! will price it — while one false positive refuses work the author knew was
//! correct, and the response to that is to delete the hook from
//! `settings.json`, which switches off every rule at once.
//!
//! So precision needs evidence, and evidence needs a human. The loop is:
//!
//! ```text
//! amont-agent explain <rule> --format cases >> crates/greenbelay-guard/tests/corpus/<rule>.cases
//! $EDITOR crates/greenbelay-guard/tests/corpus/<rule>.cases      # turn each `?` into match / nomatch
//! amont-agent corpus check               # and it is now a test
//! ```
//!
//! **The review output IS the corpus format.** There is no separate "mark as
//! reviewed" tool, because a review workflow with two file formats is one
//! nobody completes.
//!
//! ## Why a file and not a metric
//!
//! Tracking precision over time would chart the regression. A checked-in file
//! of labelled judgements *prevents* it: `corpus check` runs in the test suite,
//! so widening a rule in a way that breaks a judgement somebody already made is
//! a red build, not a number that drifts while nobody is looking.
//!
//! ## One line per case
//!
//! 35% of real commands span several lines, so newlines and tabs are escaped on
//! the way in and restored on the way out. The escaping is deliberately the
//! smallest thing that round-trips, because a corpus nobody can read by eye is
//! a corpus nobody will label.

use std::path::{Path, PathBuf};

pub const HEADER: &str = crate::names::CORPUS_HEADER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The rule should fire here.
    Match,
    /// The rule must stay silent here. These are the ones that matter — a
    /// corpus of forty positives and no negatives has an unmeasured precision,
    /// not a perfect one.
    NoMatch,
    /// Emitted by `explain`, meaning nobody has looked yet. Never counts as
    /// evidence.
    Unreviewed,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Match => "match",
            Verdict::NoMatch => "nomatch",
            Verdict::Unreviewed => "?",
        }
    }
    fn parse(s: &str) -> Option<Verdict> {
        match s {
            "match" => Some(Verdict::Match),
            "nomatch" => Some(Verdict::NoMatch),
            "?" => Some(Verdict::Unreviewed),
            _ => None,
        }
    }
}

pub struct Case {
    pub verdict: Verdict,
    pub command: String,
    pub line: usize,
}

/// Where a rule's reviewed cases live IN A CHECKOUT — the file `explain
/// --format cases` appends to and `corpus check` reads in the test suite.
///
/// `CARGO_MANIFEST_DIR` is the path of the machine that BUILT the binary.
/// For a release that is a CI runner, and the path does not exist anywhere
/// the binary later runs — so `graduate` saw "0 reviewed cases" for every
/// rule from every installed copy, and refused every promotion for lack of
/// evidence that was sitting in the repository all along. [`read`] falls
/// back to the copy compiled in; this path is for writing.
pub fn path_for(rule: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join(format!("{rule}.cases"))
}

/// Where a rule's Stop fixtures live IN A CHECKOUT: the directory `explain
/// --format cases` writes to by default. Like [`path_for`], it names the
/// machine that built the binary; the compiled-in copy is `EMBEDDED_STOP`.
pub fn stop_dir_for(rule: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join("stop")
        .join(rule)
}

/// Every rule's reviewed cases, compiled into the binary at the version
/// they were reviewed for. A release carries its own evidence, so
/// `graduate` on an installed copy judges against the same corpus the test
/// suite did — not against a path on somebody else's machine.
///
/// One entry per rule, by hand: `include_str!` refuses to compile when a
/// file is missing, and `every_rule_has_an_embedded_corpus` refuses when an
/// entry is.
const EMBEDDED: &[(&str, &str)] = &[
    (
        "workspace-root-write",
        include_str!("../tests/corpus/workspace-root-write.cases"),
    ),
    (
        "pipe-to-tail",
        include_str!("../tests/corpus/pipe-to-tail.cases"),
    ),
    (
        "bare-stash-pop",
        include_str!("../tests/corpus/bare-stash-pop.cases"),
    ),
    (
        "cd-carries-over",
        include_str!("../tests/corpus/cd-carries-over.cases"),
    ),
    (
        "gh-pr-merge-auto",
        include_str!("../tests/corpus/gh-pr-merge-auto.cases"),
    ),
    (
        "block-reinstate",
        include_str!("../tests/corpus/block-reinstate.cases"),
    ),
    ("no-verify", include_str!("../tests/corpus/no-verify.cases")),
    (
        "stdin-hang",
        include_str!("../tests/corpus/stdin-hang.cases"),
    ),
    (
        "glob-in-flag-value",
        include_str!("../tests/corpus/glob-in-flag-value.cases"),
    ),
    (
        "glob-no-match",
        include_str!("../tests/corpus/glob-no-match.cases"),
    ),
    (
        "unsplit-expansion",
        include_str!("../tests/corpus/unsplit-expansion.cases"),
    ),
    (
        "zsh-unquoted-word-split",
        include_str!("../tests/corpus/zsh-unquoted-word-split.cases"),
    ),
    (
        "equals-separator",
        include_str!("../tests/corpus/equals-separator.cases"),
    ),
    (
        "path-operand-missing",
        include_str!("../tests/corpus/path-operand-missing.cases"),
    ),
    (
        "stat-bsd-format",
        include_str!("../tests/corpus/stat-bsd-format.cases"),
    ),
    (
        "whole-file-dump",
        include_str!("../tests/corpus/whole-file-dump.cases"),
    ),
    (
        "persisted-output-dump",
        include_str!("../tests/corpus/persisted-output-dump.cases"),
    ),
    (
        "file-reread",
        include_str!("../tests/corpus/file-reread.cases"),
    ),
    (
        "clear-at-boundary",
        include_str!("../tests/corpus/clear-at-boundary.cases"),
    ),
    (
        "plan-phases-open",
        include_str!("../tests/corpus/plan-phases-open.cases"),
    ),
    (
        "read-unbounded-large",
        include_str!("../tests/corpus/read-unbounded-large.cases"),
    ),
    (
        "code-read-not-lsp",
        include_str!("../tests/corpus/code-read-not-lsp.cases"),
    ),
    (
        "symbol-grep-not-lsp",
        include_str!("../tests/corpus/symbol-grep-not-lsp.cases"),
    ),
    (
        "screenshot-in-main-session",
        include_str!("../tests/corpus/screenshot-in-main-session.cases"),
    ),
    (
        "relais-run-forks",
        include_str!("../tests/corpus/relais-run-forks.cases"),
    ),
    (
        "agent-preferred-model",
        include_str!("../tests/corpus/agent-preferred-model.cases"),
    ),
    (
        "lint-suppression-added",
        include_str!("../tests/corpus/lint-suppression-added.cases"),
    ),
    (
        "adr-lint-written",
        include_str!("../tests/corpus/adr-lint-written.cases"),
    ),
    (
        "agent-guidance-written",
        include_str!("../tests/corpus/agent-guidance-written.cases"),
    ),
    (
        "agent-guidance-shape",
        include_str!("../tests/corpus/agent-guidance-shape.cases"),
    ),
    (
        "work-overlap",
        include_str!("../tests/corpus/work-overlap.cases"),
    ),
    (
        "git-add-broad",
        include_str!("../tests/corpus/git-add-broad.cases"),
    ),
    (
        "stale-base",
        include_str!("../tests/corpus/stale-base.cases"),
    ),
    (
        "push-preflight",
        include_str!("../tests/corpus/push-preflight.cases"),
    ),
    (
        "push-preview",
        include_str!("../tests/corpus/push-preview.cases"),
    ),
    (
        "push-browser-check",
        include_str!("../tests/corpus/push-browser-check.cases"),
    ),
    (
        "ask-unopened-page",
        include_str!("../tests/corpus/ask-unopened-page.cases"),
    ),
    (
        "plan-review-panel",
        include_str!("../tests/corpus/plan-review-panel.cases"),
    ),
    (
        "plan-names-no-paths",
        include_str!("../tests/corpus/plan-names-no-paths.cases"),
    ),
    (
        "implementation-review",
        include_str!("../tests/corpus/implementation-review.cases"),
    ),
    (
        "foreground-poll",
        include_str!("../tests/corpus/foreground-poll.cases"),
    ),
    (
        "unbounded-background-push",
        include_str!("../tests/corpus/unbounded-background-push.cases"),
    ),
    (
        "gui-open-unbounded",
        include_str!("../tests/corpus/gui-open-unbounded.cases"),
    ),
    (
        "forge-merge-by-hand",
        include_str!("../tests/corpus/forge-merge-by-hand.cases"),
    ),
    (
        "release-tag-push",
        include_str!("../tests/corpus/release-tag-push.cases"),
    ),
    (
        "publish-without-skill",
        include_str!("../tests/corpus/publish-without-skill.cases"),
    ),
    (
        "sed-in-place",
        include_str!("../tests/corpus/sed-in-place.cases"),
    ),
    (
        "kubectl-gitops",
        include_str!("../tests/corpus/kubectl-gitops.cases"),
    ),
    (
        "chain-with-and",
        include_str!("../tests/corpus/chain-with-and.cases"),
    ),
    (
        "tag-after-commit",
        include_str!("../tests/corpus/tag-after-commit.cases"),
    ),
    (
        "worktree-remove-force",
        include_str!("../tests/corpus/worktree-remove-force.cases"),
    ),
    (
        "amend-pushed",
        include_str!("../tests/corpus/amend-pushed.cases"),
    ),
    (
        "branch-force-delete",
        include_str!("../tests/corpus/branch-force-delete.cases"),
    ),
    (
        "worktree-isolation",
        include_str!("../tests/corpus/worktree-isolation.cases"),
    ),
    (
        "test-output-trimmed",
        include_str!("../tests/corpus/test-output-trimmed.cases"),
    ),
    (
        "heavy-build-unqueued",
        include_str!("../tests/corpus/heavy-build-unqueued.cases"),
    ),
    (
        "land-detached",
        include_str!("../tests/corpus/land-detached.cases"),
    ),
    (
        "suppressions-override",
        include_str!("../tests/corpus/suppressions-override.cases"),
    ),
    (
        "secret-read",
        include_str!("../tests/corpus/secret-read.cases"),
    ),
];

/// The compiled-in corpus for a rule, if the table has one.
pub fn embedded(rule: &str) -> Option<&'static str> {
    EMBEDDED
        .iter()
        .find(|(id, _)| *id == rule)
        .map(|(_, text)| *text)
}

/// Is the checkout this binary was built from present here — so that
/// `explain --format cases` can name a file worth appending to?
pub fn checkout_present() -> bool {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .is_dir()
}

pub fn escape(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    for c in command.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

pub fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            // An escape we do not know is kept verbatim rather than eaten, so
            // a hand-edited file cannot silently lose a character.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

pub fn line_for(verdict: Verdict, command: &str) -> String {
    format!("{}\t{}\n", verdict.as_str(), escape(command))
}

pub fn parse(text: &str) -> Vec<Case> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((verdict, command)) = line.split_once('\t') else {
            continue;
        };
        let Some(verdict) = Verdict::parse(verdict.trim()) else {
            continue;
        };
        out.push(Case {
            verdict,
            command: unescape(command),
            line: i + 1,
        });
    }
    out
}

/// The reviewed cases for a rule: the checkout's file when this binary is
/// running from one (so a line just appended counts), else the copy
/// compiled in.
pub fn read(rule: &str) -> Vec<Case> {
    let path = path_for(rule);
    if path.exists() {
        return read_at(&path);
    }
    embedded(rule).map(parse).unwrap_or_default()
}

pub fn read_at(path: &Path) -> Vec<Case> {
    std::fs::read_to_string(path)
        .map(|t| parse(&t))
        .unwrap_or_default()
}

/// One rule's agreement with the judgements already made about it.
pub struct Score {
    pub reviewed: usize,
    pub negatives: usize,
    pub unreviewed: usize,
    /// Cases where the engine disagrees with a human. Each is either a rule
    /// that regressed or a judgement that needs revisiting; both need a person.
    pub disagreements: Vec<Disagreement>,
}

/// Why a recorded Stop fixture cannot be replayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureError {
    /// The case line's name is not a valid `Stop <name>`.
    BadCaseLine(String),
    /// No fixture is compiled in or on disk at this path.
    NoFixture(String),
    NotJson(String),
    /// A member the replay needs is absent; names it.
    Missing(String),
    /// A member has another type or value than the replay accepts.
    WrongType(String),
    /// The exporter marked the fixture not countable, with its reason.
    NotCountable(String),
}

impl std::fmt::Display for FixtureError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FixtureError::BadCaseLine(why) => write!(out, "{why}"),
            FixtureError::NoFixture(path) => write!(out, "no fixture at {path}"),
            FixtureError::NotJson(why) => write!(out, "fixture is not JSON: {why}"),
            FixtureError::Missing(member) => write!(out, "fixture has no {member}"),
            FixtureError::WrongType(why) => write!(out, "fixture member is invalid: {why}"),
            FixtureError::NotCountable(why) => write!(out, "not countable: {why}"),
        }
    }
}

/// Why the engine and the human differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisagreementReason {
    FixtureMalformed(FixtureError),
    /// The fixture is marked not countable, with the exporter's reason.
    FixtureExcluded(String),
    NoStopJudge {
        rule: String,
    },
    FiredAtDeny,
    DidNotFireAtDeny,
    Fired,
    DidNotFire,
    ReadMalformed(String),
    NoReadJudge,
}

impl std::fmt::Display for DisagreementReason {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisagreementReason::FixtureMalformed(error) => {
                write!(out, "fixture malformed: {error}")
            }
            DisagreementReason::FixtureExcluded(why) => write!(out, "fixture excluded: {why}"),
            DisagreementReason::NoStopJudge { rule } => write!(
                out,
                "no Stop judge for {rule} in this build; remove the line or label it after the judge lands"
            ),
            DisagreementReason::FiredAtDeny => write!(out, "rule fired at deny"),
            DisagreementReason::DidNotFireAtDeny => write!(out, "rule did not fire at deny"),
            DisagreementReason::Fired => write!(out, "rule fired"),
            DisagreementReason::DidNotFire => write!(out, "rule did not fire"),
            DisagreementReason::ReadMalformed(why) => write!(out, "Read case malformed: {why}"),
            DisagreementReason::NoReadJudge => write!(out, "no Read judge for this rule"),
        }
    }
}

pub struct Disagreement {
    pub line: usize,
    pub expected: Verdict,
    pub command: String,
    /// Why the engine and the human differ.
    pub why: DisagreementReason,
    /// For a Stop case, the fixture's path under `tests/corpus/`.
    pub fixture: Option<String>,
}

impl Score {
    pub fn agrees(&self) -> bool {
        self.disagreements.is_empty()
    }
    /// True positives over everything the rule claimed. `None` when nothing
    /// was claimed — an unmeasured precision, which is not the same as 1.0.
    pub fn precision(&self) -> Option<f64> {
        let claimed = self
            .disagreements
            .iter()
            .filter(|d| d.expected == Verdict::NoMatch)
            .count();
        let matched = self.reviewed - self.negatives;
        let total = matched + claimed;
        if total == 0 {
            None
        } else {
            Some(matched as f64 / total as f64)
        }
    }
}

/// Run one rule's cases through the engine as it stands today.
pub fn score(rule: &crate::rules::Rule) -> Score {
    score_with(rule, None)
}

/// [`score`] with a judge for Stop cases.
pub fn score_with(rule: &crate::rules::Rule, stop: Option<StopJudge>) -> Score {
    score_cases_with(rule, &read(rule.id), stop)
}

/// Replays one recorded Stop for a rule: the rule id and the fixture's JSON.
/// `None` when the rule has no Stop judgement; `Some(Err)` for a malformed
/// fixture; `Some(Ok(fired))` otherwise. Injected, because the replay needs
/// the process crate, which this one does not depend on.
pub type StopJudge = fn(&str, &str) -> Option<Result<bool, FixtureError>>;

/// A recorded Stop, for a rule judged from a frozen fixture. In a cases file
/// its line is `<verdict>\tStop <name>`; the fixture is
/// `tests/corpus/stop/<rule>/<name>.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopCase {
    pub name: String,
}

impl StopCase {
    /// `None` when the text is not a Stop line; `Some(Err)` when it is and
    /// the name is malformed (empty, with spaces or path separators).
    pub fn parse(text: &str) -> Option<Result<StopCase, String>> {
        let name = text.strip_prefix("Stop ")?;
        let valid = !name.is_empty()
            && name != ".."
            && name != "."
            && !name.contains(|c: char| c.is_whitespace() || c == '/' || c == '\\');
        if !valid {
            return Some(Err(
                "expected `Stop <name>`, a name without spaces or path separators".into(),
            ));
        }
        Some(Ok(StopCase {
            name: name.to_string(),
        }))
    }

    /// The fixture's path below `tests/corpus/`.
    pub fn fixture_path(&self, rule: &str) -> String {
        format!("stop/{rule}/{}.json", self.name)
    }
}

/// The largest embedded Stop fixture, in bytes: the table is compiled into
/// the binary.
pub const MAX_STOP_FIXTURE_BYTES: usize = 64 * 1024;

/// Stop fixtures compiled into the binary: `(rule, name, json)`, one entry
/// per file at `tests/corpus/stop/<rule>/<name>.json`, kept by hand like
/// [`EMBEDDED`]. `every_stop_fixture_on_disk_has_an_entry` refuses a file
/// without an entry or an entry without a file.
const EMBEDDED_STOP: &[(&str, &str, &str)] = &[
    (
        "clear-at-boundary",
        "180063a6-1794893825",
        include_str!("../tests/corpus/stop/clear-at-boundary/180063a6-1794893825.json"),
    ),
    (
        "clear-at-boundary",
        "1dbedd97-1794869349",
        include_str!("../tests/corpus/stop/clear-at-boundary/1dbedd97-1794869349.json"),
    ),
    (
        "clear-at-boundary",
        "32290a5c-1794774945",
        include_str!("../tests/corpus/stop/clear-at-boundary/32290a5c-1794774945.json"),
    ),
    (
        "clear-at-boundary",
        "32290a5c-1794907585",
        include_str!("../tests/corpus/stop/clear-at-boundary/32290a5c-1794907585.json"),
    ),
    (
        "clear-at-boundary",
        "52e82064-1794880526",
        include_str!("../tests/corpus/stop/clear-at-boundary/52e82064-1794880526.json"),
    ),
    (
        "clear-at-boundary",
        "72e46887-1794922694",
        include_str!("../tests/corpus/stop/clear-at-boundary/72e46887-1794922694.json"),
    ),
    (
        "clear-at-boundary",
        "72e46887-1794943716",
        include_str!("../tests/corpus/stop/clear-at-boundary/72e46887-1794943716.json"),
    ),
    (
        "clear-at-boundary",
        "7458c00c-1794961613",
        include_str!("../tests/corpus/stop/clear-at-boundary/7458c00c-1794961613.json"),
    ),
    (
        "clear-at-boundary",
        "8145b6c0-1794667260",
        include_str!("../tests/corpus/stop/clear-at-boundary/8145b6c0-1794667260.json"),
    ),
    (
        "clear-at-boundary",
        "8a2ca22e-1794903106",
        include_str!("../tests/corpus/stop/clear-at-boundary/8a2ca22e-1794903106.json"),
    ),
    (
        "clear-at-boundary",
        "960ad27d-1794783012",
        include_str!("../tests/corpus/stop/clear-at-boundary/960ad27d-1794783012.json"),
    ),
    (
        "clear-at-boundary",
        "a3171864-1794929214",
        include_str!("../tests/corpus/stop/clear-at-boundary/a3171864-1794929214.json"),
    ),
    (
        "clear-at-boundary",
        "c2449a0f-1794742061",
        include_str!("../tests/corpus/stop/clear-at-boundary/c2449a0f-1794742061.json"),
    ),
    (
        "clear-at-boundary",
        "c2449a0f-1794836880",
        include_str!("../tests/corpus/stop/clear-at-boundary/c2449a0f-1794836880.json"),
    ),
    (
        "clear-at-boundary",
        "e2fb2812-1794745202",
        include_str!("../tests/corpus/stop/clear-at-boundary/e2fb2812-1794745202.json"),
    ),
    (
        "clear-at-boundary",
        "e2fb2812-1794806880",
        include_str!("../tests/corpus/stop/clear-at-boundary/e2fb2812-1794806880.json"),
    ),
];

fn embedded_stop(rule: &str, name: &str) -> Option<&'static str> {
    EMBEDDED_STOP
        .iter()
        .find(|(id, fixture, _)| *id == rule && *fixture == name)
        .map(|(_, _, text)| *text)
}

/// A Read tool call, for a rule answered from the Read payload and a stat
/// rather than a command string. In a cases file its line is
/// `<verdict>\tRead <window> <bytes> <agent> <path>`: `window` is `full` or
/// `<offset>:<limit>` as the hook records it, `bytes` the file's size (a
/// regular file), `agent` is `main`, `-` for a subagent of unknown type, or
/// its `agent_type`, and the path runs to the end of the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadCase {
    pub window: String,
    pub bytes: u64,
    pub agent: crate::lsp::Agent,
    pub path: String,
}

impl ReadCase {
    /// `None` when the text is not a Read case; `Some(Err)` when it starts
    /// like one and is malformed, so a typo is a disagreement, never a case
    /// silently replayed as a shell command.
    pub fn parse(text: &str) -> Option<Result<ReadCase, String>> {
        let rest = text.strip_prefix("Read ")?;
        let mut fields = rest.splitn(4, ' ');
        let (Some(window), Some(bytes), Some(agent), Some(path)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Some(Err("expected `Read <window> <bytes> <agent> <path>`".into()));
        };
        let Ok(bytes) = bytes.parse::<u64>() else {
            return Some(Err(format!("`{bytes}` is not a size in bytes")));
        };
        if path.is_empty() {
            return Some(Err("no path".into()));
        }
        let agent = match agent {
            "main" => crate::lsp::Agent::Main,
            "-" => crate::lsp::Agent::Unnamed,
            kind => crate::lsp::Agent::Named(kind.to_string()),
        };
        Some(Ok(ReadCase {
            window: window.to_string(),
            bytes,
            agent,
            path: path.to_string(),
        }))
    }
}

/// The rules answered from a Read payload, by id, with the judgement the
/// hook itself makes. A rule absent here has no Read cases: one in its file
/// is a disagreement.
fn read_judge(rule: &str) -> Option<fn(&ReadCase) -> bool> {
    match rule {
        "read-unbounded-large" => Some(crate::rules::read_unbounded_large::judge_case),
        "secret-read" => Some(crate::rules::secret_read::judge_case),
        _ => None,
    }
}

pub fn score_cases(rule: &crate::rules::Rule, cases: &[Case]) -> Score {
    score_cases_with(rule, cases, None)
}

pub fn score_cases_with(
    rule: &crate::rules::Rule,
    cases: &[Case],
    stop: Option<StopJudge>,
) -> Score {
    score_cases_in(rule, cases, stop, &|name| {
        embedded_stop(rule.id, name).map(str::to_string)
    })
}

/// [`score_cases_with`], reading a Stop case's fixture from `fixture_named`
/// instead of the embedded table.
pub fn score_cases_in(
    rule: &crate::rules::Rule,
    cases: &[Case],
    stop: Option<StopJudge>,
    fixture_named: &dyn Fn(&str) -> Option<String>,
) -> Score {
    let mut score = Score {
        reviewed: 0,
        negatives: 0,
        unreviewed: 0,
        disagreements: Vec::new(),
    };
    for case in cases {
        if case.verdict == Verdict::Unreviewed {
            score.unreviewed += 1;
            continue;
        }
        score.reviewed += 1;
        if case.verdict == Verdict::NoMatch {
            score.negatives += 1;
        }
        let expected = case.verdict == Verdict::Match;
        let mut fixture = None;
        // `Err` is a judgement that cannot be made: a malformed Stop or Read
        // case, or one in the file of a rule with no judge for it. Either
        // disagrees, whatever its label.
        let outcome: Result<bool, DisagreementReason> =
            if let Some(stop_case) = StopCase::parse(&case.command) {
                match stop_case {
                    Err(why) => Err(DisagreementReason::FixtureMalformed(
                        FixtureError::BadCaseLine(why),
                    )),
                    Ok(stop_case) => {
                        fixture = Some(stop_case.fixture_path(rule.id));
                        replay_stop(rule.id, &stop_case, stop, fixture_named)
                    }
                }
            } else {
                match ReadCase::parse(&case.command) {
                    Some(Ok(read)) => match read_judge(rule.id) {
                        Some(judge) => Ok(judge(&read)),
                        None => Err(DisagreementReason::NoReadJudge),
                    },
                    Some(Err(why)) => Err(DisagreementReason::ReadMalformed(why)),
                    None => {
                        let parsed = crate::shell::lex(&case.command);
                        // Replayed as the backtester replays: no shell is recorded
                        // for a case, so the dialect is unknown.
                        let input = crate::rules::Input::new(
                            &case.command,
                            crate::rules::Dialect::Unknown,
                            &parsed,
                        );
                        Ok(rule.judge(&input).is_some())
                    }
                }
            };
        let is_stop = fixture.is_some();
        let why = match outcome {
            Ok(fired) if fired == expected => continue,
            Ok(true) if is_stop => DisagreementReason::FiredAtDeny,
            Ok(false) if is_stop => DisagreementReason::DidNotFireAtDeny,
            Ok(true) => DisagreementReason::Fired,
            Ok(false) => DisagreementReason::DidNotFire,
            Err(why) => why,
        };
        score.disagreements.push(Disagreement {
            line: case.line,
            expected: case.verdict,
            command: case.command.clone(),
            why,
            fixture,
        });
    }
    score
}

/// The replay of one Stop case: whether the rule fires, or why it cannot be
/// judged.
fn replay_stop(
    rule_id: &str,
    stop_case: &StopCase,
    stop: Option<StopJudge>,
    fixture_named: &dyn Fn(&str) -> Option<String>,
) -> Result<bool, DisagreementReason> {
    let no_judge = || DisagreementReason::NoStopJudge {
        rule: rule_id.to_string(),
    };
    let judge = stop.ok_or_else(no_judge)?;
    let text = fixture_named(&stop_case.name).ok_or_else(|| {
        DisagreementReason::FixtureMalformed(FixtureError::NoFixture(
            stop_case.fixture_path(rule_id),
        ))
    })?;
    match judge(rule_id, &text) {
        None => Err(no_judge()),
        Some(Err(FixtureError::NotCountable(why))) => Err(DisagreementReason::FixtureExcluded(why)),
        Some(Err(error)) => Err(DisagreementReason::FixtureMalformed(error)),
        Some(Ok(fired)) => Ok(fired),
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    /// A multi-line command must survive the round trip, because 35% of real
    /// commands are multi-clause scripts and a corpus that cannot hold them
    /// can only ever describe the easy half.
    #[test]
    fn a_command_survives_the_round_trip() {
        for command in [
            "git push | tail -1",
            "git commit -F- <<'MSG' 2>&1 | tail -8\nsubject\n\nbody\nMSG\n",
            "echo 'a\tb' && git push",
            "a\\nb literal backslash-n",
            "trailing backslash \\",
        ] {
            let line = line_for(Verdict::Match, command);
            assert_eq!(line.matches('\n').count(), 1, "one line per case");
            let back = parse(&line);
            assert_eq!(back.len(), 1);
            assert_eq!(back[0].command, command, "round trip changed {command:?}");
        }
    }

    #[test]
    fn comments_and_blank_lines_are_not_cases() {
        let text = format!("{HEADER}\n\n# a note\nmatch\tgit push | tail -1\n");
        let cases = parse(&text);
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].verdict, Verdict::Match);
    }

    #[test]
    fn a_corpus_under_either_header_loads() {
        for header in [HEADER, crate::names::old::CORPUS_HEADER] {
            let cases = parse(&format!("{header}\nmatch\tgit push | tail -1\n"));
            assert_eq!(cases.len(), 1, "{header}");
        }
        assert_eq!(HEADER, "# greenbelay-cases-v1");
    }

    /// An unlabelled case is not evidence. Counting `?` as agreement would let
    /// a dump from `explain` masquerade as a review nobody did.
    #[test]
    fn an_unreviewed_case_counts_as_no_evidence() {
        let rule = &crate::rules::pipe_to_tail::RULE;
        let cases = parse("?\tgit push | tail -1\n?\tgit status\n");
        let s = score_cases(rule, &cases);
        assert_eq!(s.reviewed, 0);
        assert_eq!(s.unreviewed, 2);
        assert!(s.agrees(), "nothing was claimed, so nothing can disagree");
        assert_eq!(s.precision(), None, "unmeasured, not perfect");
    }

    #[test]
    fn a_read_case_parses_its_fields_and_keeps_spaces_in_the_path() {
        let read = ReadCase::parse("Read full 20480 code-review-backend a b/c.rs")
            .expect("a read case")
            .expect("well formed");
        assert_eq!(read.window, "full");
        assert_eq!(read.bytes, 20_480);
        assert_eq!(
            read.agent,
            crate::lsp::Agent::Named("code-review-backend".into())
        );
        assert_eq!(read.path, "a b/c.rs");
        let main = ReadCase::parse("Read 1:200 9000 main x.rs")
            .unwrap()
            .unwrap();
        assert_eq!(main.agent, crate::lsp::Agent::Main);
        let unnamed = ReadCase::parse("Read full 9000 - x.rs").unwrap().unwrap();
        assert_eq!(unnamed.agent, crate::lsp::Agent::Unnamed);
        assert!(ReadCase::parse("cat x.rs").is_none(), "a command");
        assert!(ReadCase::parse("Read full big main x.rs").unwrap().is_err());
        assert!(ReadCase::parse("Read full 9000 main").unwrap().is_err());
    }

    /// A Read case is judged by the rule's own decision, and fails loudly
    /// wherever it cannot be: malformed, or in a rule with no Read judge.
    #[test]
    fn a_read_case_is_judged_by_its_rule_or_disagrees() {
        let large = &crate::rules::read_unbounded_large::RULE;
        let s = score_cases(
            large,
            &parse(
                "match\tRead full 20480 general-purpose src/a.rs\n\
                 nomatch\tRead full 20480 implementation-review src/a.rs\n\
                 nomatch\tRead full 8192 general-purpose src/a.rs\n\
                 nomatch\tcat src/a.rs\n",
            ),
        );
        assert_eq!((s.reviewed, s.negatives), (4, 3));
        assert!(
            s.agrees(),
            "{:?}",
            s.disagreements.iter().map(|d| d.line).collect::<Vec<_>>()
        );
        let bad = score_cases(large, &parse("nomatch\tRead full lots main src/a.rs\n"));
        assert_eq!(
            bad.disagreements.len(),
            1,
            "malformed disagrees, whatever its label"
        );
        let tail = &crate::rules::pipe_to_tail::RULE;
        let foreign = score_cases(tail, &parse("nomatch\tRead full 20480 main src/a.rs\n"));
        assert_eq!(
            foreign.disagreements.len(),
            1,
            "no Read judge for this rule"
        );
    }

    #[test]
    fn a_disagreement_names_the_line_and_the_command() {
        let rule = &crate::rules::pipe_to_tail::RULE;
        // A human says this must stay silent; the rule fires. That is the
        // shape of every false positive worth catching.
        let cases = parse("nomatch\tgit push origin main | tail -1\n");
        let s = score_cases(rule, &cases);
        assert!(!s.agrees());
        assert_eq!(s.disagreements[0].line, 1);
        assert_eq!(s.disagreements[0].expected, Verdict::NoMatch);
    }

    fn stop_rule_case(label: &str, name: &str) -> Vec<Case> {
        parse(&format!("{label}\tStop {name}\n"))
    }

    fn fired(_rule: &str, _fixture: &str) -> Option<Result<bool, FixtureError>> {
        Some(Ok(true))
    }

    fn malformed(_rule: &str, _fixture: &str) -> Option<Result<bool, FixtureError>> {
        Some(Err(FixtureError::NotJson("eof".into())))
    }

    #[test]
    fn a_stop_line_parses_its_name_or_is_malformed() {
        assert_eq!(
            StopCase::parse("Stop early-1").unwrap().unwrap().name,
            "early-1"
        );
        assert!(StopCase::parse("git status").is_none());
        assert!(StopCase::parse("Read full 1 main x").is_none());
        for bad in ["Stop ", "Stop a b", "Stop a/b", "Stop a\\b", "Stop .."] {
            assert!(StopCase::parse(bad).unwrap().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_stop_case_with_no_judge_is_a_disagreement() {
        let rule = &crate::rules::pipe_to_tail::RULE;
        let score = score_cases_with(rule, &stop_rule_case("nomatch", "x"), None);
        assert_eq!(score.disagreements.len(), 1);
        assert_eq!(
            score.disagreements[0].why,
            DisagreementReason::NoStopJudge {
                rule: "pipe-to-tail".to_string()
            }
        );
        assert_eq!(
            score.disagreements[0].why.to_string(),
            "no Stop judge for pipe-to-tail in this build; remove the line or label it after the judge lands"
        );
    }

    #[test]
    fn a_stop_case_labelled_against_the_judge_names_why() {
        let rule = &crate::rules::pipe_to_tail::RULE;
        let fixtures = |_: &str| Some("{}".to_string());
        let score = score_cases_in(
            rule,
            &stop_rule_case("nomatch", "x"),
            Some(fired),
            &fixtures,
        );
        assert_eq!(score.disagreements[0].why, DisagreementReason::FiredAtDeny);
        assert_eq!(score.disagreements[0].why.to_string(), "rule fired at deny");
        assert_eq!(
            score.disagreements[0].fixture.as_deref(),
            Some("stop/pipe-to-tail/x.json")
        );
        let agree = score_cases_in(rule, &stop_rule_case("match", "x"), Some(fired), &fixtures);
        assert!(agree.agrees());
        let bad = score_cases_in(
            rule,
            &stop_rule_case("match", "x"),
            Some(malformed),
            &fixtures,
        );
        assert_eq!(
            bad.disagreements[0].why,
            DisagreementReason::FixtureMalformed(FixtureError::NotJson("eof".into()))
        );
        assert_eq!(
            bad.disagreements[0].why.to_string(),
            "fixture malformed: fixture is not JSON: eof"
        );
    }

    fn stop_fixtures_on_disk() -> Vec<String> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("corpus")
            .join("stop");
        let mut found = Vec::new();
        let Ok(rules) = std::fs::read_dir(&root) else {
            return found;
        };
        for rule in rules.flatten() {
            let Ok(files) = std::fs::read_dir(rule.path()) else {
                continue;
            };
            for file in files.flatten() {
                let name = file.file_name().to_string_lossy().into_owned();
                if let Some(stem) = name.strip_suffix(".json") {
                    found.push(format!("{}/{stem}", rule.file_name().to_string_lossy()));
                }
            }
        }
        found.sort();
        found
    }

    #[test]
    fn every_stop_fixture_on_disk_has_an_entry_and_every_entry_a_file() {
        let mut table: Vec<String> = EMBEDDED_STOP
            .iter()
            .map(|(rule, name, _)| format!("{rule}/{name}"))
            .collect();
        table.sort();
        assert_eq!(stop_fixtures_on_disk(), table);
    }

    #[test]
    fn no_embedded_stop_fixture_exceeds_the_cap() {
        for (rule, name, text) in EMBEDDED_STOP {
            assert!(
                text.len() <= MAX_STOP_FIXTURE_BYTES,
                "stop/{rule}/{name}.json is {} bytes",
                text.len()
            );
        }
    }
}
