//! `greenspotter-written` — an Edit, MultiEdit or Write of a source file, checked
//! by `greenspotter check --json` the moment it lands (decisions ADR-0044,
//! behaviour 4).
//!
//! It is quiet unless the file's repository vendors greenspotter's `check` row:
//! `amont list --json` names the checks a repository declares, and that
//! document is read, never `amont.conf` (`harness.cross-tool-reads`). The row
//! is matched by its id, `pre-commit-greenspotter-check`: until the row is
//! trusted, amont lists it `unusable` with no `command`, and a vendored row
//! is adopted whether or not its commit gate may run yet.
//!
//! Everything here is a calculation over text the hook already read: which
//! files are considered, whether a gate document opens the gate, what an
//! greenspotter exit means, how its findings are phrased. The spawns, the file
//! reads and the once-per-session records are the hook's
//! (`on_post_write` in `src/hook.rs`). `examine` never fires — the backtester sees
//! no Edit calls.

use std::path::Path;

use crate::rules::{Evidence, Examine, Finding, Rule, Stance, Trend};
use crate::shell::Parsed;

pub const RULE: Rule = Rule {
    id: "greenspotter-written",
    prevents:
        "A decided code rule broken in a file just written, found only at review or commit time",
    irreversible: false,
    default_stance: Stance::Advise,
    max_stance: Stance::Advise,
    evidence: Evidence {
        per_1000: 0.0,
        measured: "2026-10-25",
        trend: Trend::Rare,
    },
    examine: Examine::Legacy(examine),
    confirm: None,
};

fn examine(_parsed: &Parsed) -> Option<Finding> {
    None
}

/// The `format` token of the document `amont list --json` prints.
pub const LIST_FORMAT: &str = "amont-list-v1";

/// The same envelope under greengate 4.0's name; both are read until the
/// census of greengate's hard rename.
pub const LIST_FORMATS: [&str; 2] = ["greengate-list-v1", LIST_FORMAT];

/// The id greengate gives the row greenspotter's pack declares, then the id
/// the same row had under adr-lint's pack: a repository the fleet sweep
/// (topo-spotter-rename t10) has not reached still vendors the old one, and
/// keeps its write-time check. Both are read until that sweep.
pub const ROW_IDS: [&str; 2] = ["pre-commit-greenspotter-check", "pre-commit-adr-lint-check"];

/// The extensions greenspotter's code rules apply to.
const EXTENSIONS: &[&str] = &["rs", "ts", "tsx", "go", "py"];

/// Is this a source file greenspotter checks?
pub fn considered(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e))
}

/// What the document `amont list --json` printed says about the gate.
#[derive(Debug, PartialEq, Eq)]
pub enum Gate {
    /// A well-formed document listing greenspotter's `check` row.
    Open,
    /// A well-formed document without one: an ordinary repository.
    Closed,
    /// Not JSON, or not the `amont-list-v1` shape: journalled, and closed.
    Unexpected,
}

pub fn gate(document: &str) -> Gate {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(document) else {
        return Gate::Unexpected;
    };
    if !doc
        .get("format")
        .and_then(|f| f.as_str())
        .is_some_and(|f| LIST_FORMATS.contains(&f))
    {
        return Gate::Unexpected;
    }
    let Some(checks) = doc.get("checks").and_then(|c| c.as_array()) else {
        return Gate::Unexpected;
    };
    let vendored = checks.iter().any(|check| {
        check
            .get("id")
            .and_then(|id| id.as_str())
            .is_some_and(|id| ROW_IDS.contains(&id))
    });
    if vendored {
        Gate::Open
    } else {
        Gate::Closed
    }
}

/// One finding of `greenspotter check --json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintFinding {
    pub path: String,
    pub line: u64,
    pub col: u64,
    pub rule: String,
    pub message: String,
    pub text: String,
}

/// What a finished greenspotter run means.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Clean,
    Findings(Vec<LintFinding>),
    /// Not shown to the agent; journalled once per session under this outcome.
    Failed(&'static str),
}

/// Reads an exit status and the stdout of `greenspotter check --json`.
pub fn interpret(exit: Option<i32>, stdout: &str) -> Verdict {
    match exit {
        Some(0) => Verdict::Clean,
        Some(1) => match parse_findings(stdout) {
            Some(findings) if !findings.is_empty() => Verdict::Findings(findings),
            _ => Verdict::Failed("greenspotter-unparsed"),
        },
        Some(2) => Verdict::Failed("greenspotter-exit-2"),
        Some(3) => Verdict::Failed("greenspotter-exit-3"),
        _ => Verdict::Failed("greenspotter-exit-other"),
    }
}

fn parse_findings(stdout: &str) -> Option<Vec<LintFinding>> {
    let doc: serde_json::Value = serde_json::from_str(stdout).ok()?;
    doc.as_array()?
        .iter()
        .map(|row| {
            let text = |key: &str| row.get(key)?.as_str().map(str::to_string);
            let number = |key: &str| row.get(key)?.as_u64();
            Some(LintFinding {
                path: text("path")?,
                line: number("line")?,
                col: number("col")?,
                rule: text("rule")?,
                message: text("message")?,
                text: text("text")?,
            })
        })
        .collect()
}

/// The journal excerpt of one finding.
pub fn excerpt(finding: &LintFinding) -> String {
    format!("{} {} {}", finding.rule, finding.path, finding.text.trim())
}

/// The advice: each finding, one `greentopo rule` line per distinct rule, and how
/// to waive.
pub fn phrase(findings: &[LintFinding]) -> String {
    let mut lines: Vec<String> = findings
        .iter()
        .map(|f| format!("{}:{}:{}: {}: {}", f.path, f.line, f.col, f.rule, f.message))
        .collect();
    let mut rules: Vec<&str> = Vec::new();
    for finding in findings {
        if !rules.contains(&finding.rule.as_str()) {
            rules.push(&finding.rule);
        }
    }
    lines.extend(rules.iter().map(|rule| format!("greentopo rule {rule}")));
    let reason = format!(
        "greenspotter found {} on the file just written:\n{}",
        if findings.len() == 1 {
            "1 finding".to_string()
        } else {
            format!("{} findings", findings.len())
        },
        lines.join("\n")
    );
    crate::decision::phrase(
        RULE.id,
        &reason,
        "To waive one on purpose, put `greenspotter: allow <rule-id> -- <reason>` on the line.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = r##"[{"path":"src/a.rs","line":3,"col":5,"rule":"tests.first-properties","message":"an ignored test needs a reason","detail":"d","text":"#[ignore]"}]"##;

    #[test]
    fn only_source_extensions_are_considered() {
        for yes in ["a.rs", "a.ts", "a.tsx", "a.go", "a.py"] {
            assert!(considered(Path::new(yes)), "{yes}");
        }
        for no in ["a.md", "a.js", "Makefile", "a.rs.bak", "a.toml"] {
            assert!(!considered(Path::new(no)), "{no}");
        }
    }

    #[test]
    fn the_gate_needs_the_format_and_the_row_id_trusted_or_not() {
        // As amont 1.x lists a vendored row before and after `amont trust`.
        let untrusted = r#"{"format":"amont-list-v1","checks":[{"id":"pre-commit-cargo-test","command":null},{"id":"pre-commit-greenspotter-check","status":"unusable","command":null}]}"#;
        let trusted = r#"{"format":"amont-list-v1","checks":[{"id":"pre-commit-greenspotter-check","status":"runs","command":"greenspotter check --"}]}"#;
        assert_eq!(gate(untrusted), Gate::Open);
        // A repository the fleet sweep has not reached lists the old id.
        assert_eq!(
            gate(&untrusted.replace(ROW_IDS[0], "pre-commit-adr-lint-check")),
            Gate::Open
        );
        assert_eq!(gate(trusted), Gate::Open);
        // greengate 4.0 names the same envelope `greengate-list-v1`.
        assert_eq!(
            gate(&trusted.replace("amont-list-v1", "greengate-list-v1")),
            Gate::Open
        );
        assert_eq!(
            gate(
                r#"{"format":"amont-list-v1","checks":[{"id":"pre-commit-mine","command":"greenspotter check --"}]}"#
            ),
            Gate::Closed
        );
        for unexpected in [
            r#"{"format":"amont-list-v2","checks":[{"id":"pre-commit-greenspotter-check"}]}"#,
            r#"{"format":"greengate-list-v2","checks":[{"id":"pre-commit-greenspotter-check"}]}"#,
            r#"{"checks":[{"id":"pre-commit-greenspotter-check"}]}"#,
            r#"{"format":"amont-list-v1"}"#,
            "not json",
        ] {
            assert_eq!(gate(unexpected), Gate::Unexpected, "{unexpected}");
        }
    }

    #[test]
    fn exit_codes_have_their_meaning() {
        assert_eq!(interpret(Some(0), ""), Verdict::Clean);
        assert!(matches!(interpret(Some(1), ONE), Verdict::Findings(f) if f.len() == 1));
        assert_eq!(
            interpret(Some(1), "oops"),
            Verdict::Failed("greenspotter-unparsed")
        );
        assert_eq!(
            interpret(Some(1), "[]"),
            Verdict::Failed("greenspotter-unparsed")
        );
        assert_eq!(
            interpret(Some(2), ""),
            Verdict::Failed("greenspotter-exit-2")
        );
        assert_eq!(
            interpret(Some(3), ""),
            Verdict::Failed("greenspotter-exit-3")
        );
        assert_eq!(
            interpret(None, ""),
            Verdict::Failed("greenspotter-exit-other")
        );
    }

    #[test]
    fn the_advice_lists_findings_and_each_rule_once() {
        let Verdict::Findings(mut findings) = interpret(Some(1), ONE) else {
            panic!("findings");
        };
        findings.push(findings[0].clone());
        let text = phrase(&findings);
        assert!(
            text.contains("src/a.rs:3:5: tests.first-properties: an ignored test needs a reason")
        );
        assert_eq!(
            text.matches("greentopo rule tests.first-properties")
                .count(),
            1
        );
        assert!(text.contains("greenspotter: allow <rule-id> -- <reason>"));
    }

    #[test]
    fn a_row_missing_a_field_is_unparsed() {
        assert_eq!(
            interpret(Some(1), r#"[{"path":"a.rs"}]"#),
            Verdict::Failed("greenspotter-unparsed")
        );
    }
}
