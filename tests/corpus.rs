//! The reviewed judgements, run as a test.
//!
//! This is the whole point of keeping a corpus instead of a precision metric. A
//! metric would chart the regression; this prevents it. Widening a rule in a way
//! that breaks a judgement somebody already made is a red build, not a number
//! that drifts while nobody is looking.
//!
//! Every `match` line came out of the real transcripts via
//! `amont-agent explain <rule> --format cases` and was then labelled by hand.
//! Every `nomatch` line is a shape that a naive version of some rule DID match
//! while this was being measured.

use std::process::Command;

fn corpus_check() -> (i32, String) {
    let out = greenpiste_command()
        .args(["corpus", "check"])
        .output()
        .expect("the binary runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn every_reviewed_judgement_still_holds() {
    let (code, out) = corpus_check();
    assert_eq!(
        code, 0,
        "a rule no longer agrees with a reviewed case:\n{out}"
    );
}

/// A corpus of positives alone measures recall, not precision. Without
/// expected-negatives, "100%" means nobody ever gave the rule a chance to be
/// wrong.
#[test]
fn every_rule_carries_expected_negatives() {
    let (_, out) = corpus_check();
    for line in out.lines() {
        if line.contains("no cases yet") || !line.contains("reviewed") {
            continue;
        }
        let negatives: usize = line
            .split_once('(')
            .and_then(|(_, rest)| rest.split_once(' '))
            .and_then(|(n, _)| n.parse().ok())
            .unwrap_or(0);
        assert!(
            negatives >= 4,
            "too few expected-negatives to measure precision: {line}"
        );
    }
}

/// The corpus must cover every rule. A rule with no cases is a rule whose
/// precision nobody has ever checked, and adding one should be as conspicuous
/// as adding a rule without a test.
#[test]
fn every_rule_has_a_corpus() {
    let (_, out) = corpus_check();
    assert!(
        !out.contains("no cases yet"),
        "a rule has no reviewed cases:\n{out}"
    );
}

/// The greenpiste binary pointed at scratch directories, so the migration every
/// subcommand runs first never touches the real home of whoever runs the tests.
/// A caller that sets one of these variables itself overrides it afterwards.
fn greenpiste_command() -> Command {
    static SCRATCH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let scratch = SCRATCH.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("gp-corpus-scratch-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("claude")).expect("scratch config dir");
        std::fs::create_dir_all(dir.join("state")).expect("scratch state home");
        dir
    });
    let mut command = Command::new(env!("CARGO_BIN_EXE_greenpiste"));
    command
        .env("CLAUDE_CONFIG_DIR", scratch.join("claude"))
        .env("XDG_STATE_HOME", scratch.join("state"))
        .env("GIT_CONFIG_GLOBAL", scratch.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}
