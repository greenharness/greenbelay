//! A test or build command ran. Did it change a home file it had no business
//! touching?
//!
//! 2026-11-15: a leaky test run migrated the real home: it rewrote
//! `~/.config/git/config` and the Claude plugin files, and nothing failed. On
//! 2026-11-11 a migrating binary's tests, run by hand, touched the live
//! repository. Neither command's exit status said so. This assertion takes a
//! fingerprint of the watched set
//! ([`crate::home_print`]) before such a command and compares it after, so the
//! change is a fact in front of the model while the command is still on
//! screen.
//!
//! It advises only, by construction: an assertion cannot refuse, and the
//! person decided this net warns and never blocks. A file the person edited
//! while the command ran shows too, and the advice says so.

use crate::assertions::{Assertion, Claim, Verdict};
use crate::home_print;
use crate::payload::Bash;
use crate::rules::heavy_build_unqueued::is_heavy;
use crate::rules::test_output_trimmed::{exempt_words, is_check_run};
use crate::rules::{Context, Evidence, Stance, Trend};
use crate::shell::{Parsed, Simple};

pub const ASSERTION: Assertion = Assertion {
    id: "home-fingerprint-drift",
    prevents:
        "A test or build command that changed a home or repository config file going unnoticed",
    irreversible: false,
    default_stance: Stance::Advise,
    // The rate is how often the claim is MADE (a test or build command ran),
    // not how often a home file changed: 1,221 of 32,123 Bash calls,
    // `greenbelay backtest --rule home-fingerprint-drift --since 2026-11-13`.
    // Each costs about 3 ms of hook time on each side (measured at load 266).
    evidence: Evidence {
        per_1000: 38.0,
        measured: "2026-11-20",
        trend: Trend::Routine,
    },
    examine,
    verify,
};

/// The first clause that runs tests, a build, or a binary out of `target/`.
fn trigger(parsed: &Parsed) -> Option<&Simple> {
    parsed.judgeable().find(|cmd| triggers(cmd))
}

fn triggers(cmd: &Simple) -> bool {
    let Some(program) = cmd.program() else {
        return false;
    };
    if exempt_words(cmd.args()) {
        return false;
    }
    is_check_run(cmd) || is_heavy(program, cmd.args()) || built_binary(program) || queued(cmd)
}

/// A binary run straight out of a cargo build: `./target/debug/x`.
fn built_binary(program: &str) -> bool {
    let p = program.replace('\\', "/");
    ["target/debug/", "target/release/"]
        .iter()
        .any(|t| p.starts_with(t) || p.contains(&format!("/{t}")))
}

/// A run behind a wrapper: `greengate slot run … -- cargo test` (queued, but a
/// test run all the same) or `greenbelay quiet -- ./target/debug/x`.
fn queued(cmd: &Simple) -> bool {
    let Some(program) = cmd.program() else {
        return false;
    };
    let args = cmd.args();
    let Some(sep) = args.iter().position(|w| !w.quoted && w.text == "--") else {
        return false;
    };
    let mut own = args[..sep]
        .iter()
        .filter(|w| !w.quoted && !w.text.starts_with('-'))
        .map(|w| w.text.as_str());
    let wraps = if crate::gate::is_gate_program(program) {
        own.next() == Some("slot") && own.next() == Some("run")
    } else {
        crate::names::is_ours_program(program) && own.next() == Some("quiet")
    };
    if !wraps {
        return false;
    }
    match args[sep + 1..].split_first() {
        Some((p, rest)) if !p.quoted => {
            !exempt_words(rest) && (is_heavy(&p.text, rest) || built_binary(&p.text))
        }
        _ => false,
    }
}

fn examine(parsed: &Parsed) -> Option<Claim> {
    let cmd = trigger(parsed)?;
    Some(Claim {
        what: "home files unchanged".to_string(),
        span: cmd.at..cmd.end,
    })
}

/// PreToolUse, for a command that is going to run in the foreground: keep
/// the fingerprint the PostToolUse compares against. Silent and best-effort:
/// a snapshot that cannot be kept leaves the assertion `unverified`.
pub fn before(bash: &Bash) {
    if bash.background || !home_print::usable_id(&bash.tool_use_id) {
        return;
    }
    let parsed = crate::shell::lex(&bash.command);
    let Some(cmd) = trigger(&parsed) else {
        return;
    };
    let ctx = Context {
        platform: None,
        cwd: &bash.cwd,
        parsed: &parsed,
        background: bash.background,
        timeout_ms: bash.timeout_ms,
        tool_use_id: &bash.tool_use_id,
        transcript: None,
        agent_transcript: None,
        command: &bash.command,
        output: None,
        session: &bash.session,
        failed: false,
    };
    let git_dir = home_print::git_common_dir(&ctx.cwd_at(cmd.at));
    let roots = home_print::watched(&|k| std::env::var_os(k), git_dir.as_deref());
    let snap = home_print::snapshot(&roots);
    if let Some(store) = home_print::store() {
        if let Err(e) = home_print::keep(&store, &bash.tool_use_id, &snap) {
            eprintln!(
                concat!(
                    crate::bin!(),
                    ": home-fingerprint-drift: could not keep the snapshot: {}"
                ),
                e
            );
        }
    }
}

fn verify(ctx: &Context, claim: &Claim) -> Verdict {
    let Some(store) = home_print::store() else {
        return Verdict::Unknown("no state directory");
    };
    let Some(before) = home_print::take(&store, ctx.tool_use_id) else {
        return Verdict::Unknown("no snapshot was kept before the command");
    };
    let after = home_print::as_recorded(&home_print::snapshot(&before.roots));
    let changed = home_print::diff(&before, &after);
    if changed.is_empty() {
        return Verdict::Held;
    }
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from);
    Verdict::Broken {
        reason: reason(ctx.command, claim, &changed, home.as_deref()),
        remedy: "Inspect each file (`git config --list --show-origin` for git); if this \
                 command changed it, restore it and run the tests again with `HOME` and \
                 `XDG_CONFIG_HOME` set to a scratch directory."
            .to_string(),
    }
}

/// The longest command excerpt the reason quotes.
const SHOWN: usize = 60;

fn reason(
    command: &str,
    claim: &Claim,
    changed: &[(std::path::PathBuf, home_print::Change)],
    home: Option<&std::path::Path>,
) -> String {
    let clause = command
        .get(claim.span.clone())
        .unwrap_or(command)
        .trim()
        .replace(['\n', '\r'], " ");
    let shown: String = if clause.chars().count() > SHOWN {
        let mut s: String = clause.chars().take(SHOWN - 1).collect();
        s.push('…');
        s
    } else {
        clause
    };
    let mut out = format!("These files changed while `{shown}` ran:");
    for (p, c) in changed {
        out.push_str(&format!(
            "\n  {} ({})",
            home_print::display(p, home),
            c.as_str()
        ));
    }
    out.push_str("\nAn edit you made at the same time shows here too.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::lex;

    fn claims(command: &str) -> bool {
        examine(&lex(command)).is_some()
    }

    #[test]
    fn test_and_build_commands_are_claims() {
        assert!(claims("cargo test"));
        assert!(claims("cargo test --workspace --locked"));
        assert!(claims("cargo build --release"));
        assert!(claims("pnpm test"));
        assert!(claims("python3 -m pytest -q"));
        assert!(claims("make check"));
        assert!(claims("cd crates/x && cargo nextest run"));
        assert!(claims("greengate slot run -- cargo test"));
        assert!(claims("amont run cargo-test"));
    }

    #[test]
    fn a_binary_out_of_target_is_a_claim() {
        assert!(claims("./target/debug/greenbelay migrate"));
        assert!(claims("target/release/relais run x"));
        assert!(claims("/Users/me/repo/target/debug/tool --flag"));
        assert!(claims("greengate slot run -- ./target/debug/tool"));
    }

    #[test]
    fn help_listing_and_unrelated_commands_are_not() {
        assert!(!claims("cargo test --help"));
        assert!(!claims("cargo test --no-run"));
        assert!(!claims("cargo nextest list --list"));
        assert!(!claims("./target/debug/greenbelay --version"));
        assert!(!claims("cargo fmt"));
        assert!(!claims("git status"));
        assert!(!claims("ls target/debug/"));
        assert!(!claims("echo cargo test"));
        assert!(!claims("mytarget/debug/x"));
    }

    #[test]
    fn the_span_is_the_triggering_clause() {
        let command = "git status && cargo test -p x";
        let claim = examine(&lex(command)).expect("a claim");
        assert_eq!(command[claim.span].trim(), "cargo test -p x");
    }

    #[test]
    fn the_reason_lists_one_path_per_line_and_says_a_person_edit_shows() {
        let claim = Claim {
            what: String::new(),
            span: 0..10,
        };
        let changed = vec![
            (
                std::path::PathBuf::from("/h/.gitconfig"),
                home_print::Change::Modified,
            ),
            (
                std::path::PathBuf::from("/r/.git/config"),
                home_print::Change::Added,
            ),
        ];
        let r = reason(
            "cargo test",
            &claim,
            &changed,
            Some(std::path::Path::new("/h")),
        );
        let lines: Vec<&str> = r.lines().collect();
        assert_eq!(lines[0], "These files changed while `cargo test` ran:");
        assert!(lines[1].ends_with(".gitconfig (modified)"), "{r}");
        assert!(lines[2].ends_with("config (added)"), "{r}");
        assert!(lines[3].contains("at the same time"));
    }

    /// The reviewed cases in `tests/corpus/home-fingerprint-drift.cases`
    /// replay through `examine`: the corpus check covers rules, and this is
    /// the same file format checked for an assertion.
    #[test]
    fn every_reviewed_case_still_holds() {
        let text = include_str!("../../tests/corpus/home-fingerprint-drift.cases");
        let cases = crate::corpus::parse(text);
        let negatives = cases
            .iter()
            .filter(|c| c.verdict == crate::corpus::Verdict::NoMatch)
            .count();
        assert!(negatives >= 5, "expected negatives: {negatives}");
        for c in &cases {
            let want = match c.verdict {
                crate::corpus::Verdict::Match => true,
                crate::corpus::Verdict::NoMatch => false,
                crate::corpus::Verdict::Unreviewed => continue,
            };
            assert_eq!(claims(&c.command), want, "line {}: {}", c.line, c.command);
        }
    }
}
