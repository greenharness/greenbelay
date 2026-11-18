//! Stop cases, scored with the real Stop judge.
//!
//! A Stop case is a frozen recorded Stop (`Stop <name>` in a rule's cases
//! file, the fixture under `tests/corpus/stop/<rule>/`). The embedded corpora
//! must agree with the judge; hand-made fixtures show the failure shapes.

use greenbelay_guard::corpus::{self, DisagreementReason, FixtureError};
use greenbelay_guard::rules::clear_at_boundary::RULE;
use greenbelay_guard::rules::Rule;
use greenbelay_process::clear_boundary::stop_judge;
use std::process::Command;

fn clear_at_boundary() -> &'static Rule {
    &RULE
}

/// A new, empty scratch directory for one spawn, so the migration the binary
/// runs first never touches the real home of whoever runs the tests.
fn fresh_scratch(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mut suffix = 0u32;
    loop {
        let dir = std::env::temp_dir().join(format!(
            "gp-{label}-{}-{nanos}-{suffix}",
            std::process::id()
        ));
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                std::fs::create_dir(dir.join("claude")).expect("scratch config dir");
                std::fs::create_dir(dir.join("state")).expect("scratch state home");
                return dir;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => suffix += 1,
            Err(error) => panic!("scratch directory {}: {error}", dir.display()),
        }
    }
}

/// `corpus check` is wired with the real Stop judge, so the binary scores
/// every embedded corpus, Stop cases included.
#[test]
fn every_embedded_corpus_agrees_with_the_real_stop_judge() {
    let scratch = fresh_scratch("corpus-stop");
    let output = Command::new(env!("CARGO_BIN_EXE_greenbelay"))
        .env("CLAUDE_CONFIG_DIR", scratch.join("claude"))
        .env("XDG_STATE_HOME", scratch.join("state"))
        .env("GIT_CONFIG_GLOBAL", scratch.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["corpus", "check"])
        .output()
        .expect("the binary runs");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{text}");
}

#[test]
fn a_fixture_labelled_the_wrong_way_names_the_line_the_path_and_the_firing() {
    let rule = clear_at_boundary();
    let cases = corpus::parse("match\tStop quiet\nnomatch\tStop loud\n");
    let fixtures = |_: &str| Some("{}".to_string());
    let always_fires: corpus::StopJudge = |_, _| Some(Ok(true));
    let score = corpus::score_cases_in(rule, &cases, Some(always_fires), &fixtures);
    assert_eq!(score.disagreements.len(), 1);
    let disagreement = &score.disagreements[0];
    assert_eq!(disagreement.line, 2);
    assert_eq!(disagreement.command, "Stop loud");
    assert_eq!(
        disagreement.fixture.as_deref(),
        Some("stop/clear-at-boundary/loud.json")
    );
    assert_eq!(disagreement.why, DisagreementReason::FiredAtDeny);

    let never_fires: corpus::StopJudge = |_, _| Some(Ok(false));
    let score = corpus::score_cases_in(rule, &cases, Some(never_fires), &fixtures);
    assert_eq!(score.disagreements[0].line, 1);
    assert_eq!(
        score.disagreements[0].why,
        DisagreementReason::DidNotFireAtDeny
    );
}

#[test]
fn a_malformed_fixture_is_a_disagreement_whatever_its_label() {
    let rule = clear_at_boundary();
    let fixtures = |_: &str| Some("not json".to_string());
    for label in ["match", "nomatch"] {
        let cases = corpus::parse(&format!("{label}\tStop broken\n"));
        let score = corpus::score_cases_in(rule, &cases, Some(stop_judge), &fixtures);
        assert_eq!(score.disagreements.len(), 1, "{label}");
        assert!(
            matches!(
                score.disagreements[0].why,
                DisagreementReason::FixtureMalformed(FixtureError::NotJson(_))
            ),
            "{:?}",
            score.disagreements[0].why
        );
        assert!(score.disagreements[0]
            .why
            .to_string()
            .starts_with("fixture malformed: "));
    }
}

#[test]
fn a_fixture_missing_a_member_or_with_a_wrong_one_names_which() {
    let rule = clear_at_boundary();
    let cases = corpus::parse("match\tStop broken\n");
    let missing = |_: &str| Some("{}".to_string());
    let score = corpus::score_cases_in(rule, &cases, Some(stop_judge), &missing);
    assert!(
        matches!(
            &score.disagreements[0].why,
            DisagreementReason::FixtureMalformed(FixtureError::Missing(member)) if member == "payload"
        ),
        "{:?}",
        score.disagreements[0].why
    );
    let wrong = |_: &str| {
        Some(
            r#"{"payload": {"cwd": null, "permission_mode": null, "background_busy": false,
                "stop_hook_active": false, "ends_with_question": false,
                "waiting_line": false, "marker": false},
              "state": {"key": "", "advices": 0, "pending": null},
              "facts": {"phase": "sideways", "repositories": "clean"},
              "entries": []}"#
                .to_string(),
        )
    };
    let score = corpus::score_cases_in(rule, &cases, Some(stop_judge), &wrong);
    assert!(
        matches!(
            score.disagreements[0].why,
            DisagreementReason::FixtureMalformed(FixtureError::WrongType(_))
        ),
        "{:?}",
        score.disagreements[0].why
    );
}

#[test]
fn an_excluded_fixture_says_it_is_excluded_and_why() {
    let rule = clear_at_boundary();
    let cases = corpus::parse("match\tStop gone\n");
    let excluded = |_: &str| {
        Some(
            r#"{"payload": {"cwd": null, "permission_mode": null, "background_busy": false,
                "stop_hook_active": false, "ends_with_question": false,
                "waiting_line": false, "marker": false},
              "state": {"key": "", "advices": 0, "pending": null},
              "facts": {"phase": "none", "repositories": "clean"},
              "countable": false, "excluded": "no entry uuid=x", "entries": []}"#
                .to_string(),
        )
    };
    let score = corpus::score_cases_in(rule, &cases, Some(stop_judge), &excluded);
    assert_eq!(
        score.disagreements[0].why.to_string(),
        "fixture excluded: no entry uuid=x"
    );
}
