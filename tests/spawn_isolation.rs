//! Every integration test that starts the binary must point it at scratch
//! config, state and git config, or it migrates the real home of whoever runs
//! `cargo test`. This is the static half of `scripts/leak-hunt.sh`, which runs
//! each test file against a canary machine and sees what actually happened.

use std::path::Path;

/// Files that start the binary and genuinely cannot name the variables: the
/// file name and the reason.
const ALLOW: &[(&str, &str)] = &[(
    "names.rs",
    "only quotes the binary's variable name in a string; it spawns nothing",
)];

const REQUIRED: [&str; 4] = [
    "CLAUDE_CONFIG_DIR",
    "XDG_STATE_HOME",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_NOSYSTEM",
];

/// The variables `source` starts the binary without mentioning; empty when it
/// does not start the binary at all.
fn missing_isolation(source: &str) -> Vec<&'static str> {
    if !source.contains("CARGO_BIN_EXE_greenpiste") {
        return Vec::new();
    }
    REQUIRED
        .into_iter()
        .filter(|variable| {
            // `GIT_CONFIG_SYSTEM` (pointed at /dev/null) isolates the system
            // scope as well as `GIT_CONFIG_NOSYSTEM` does.
            let stands_in =
                *variable == "GIT_CONFIG_NOSYSTEM" && source.contains("GIT_CONFIG_SYSTEM");
            !source.contains(variable) && !stands_in
        })
        .collect()
}

#[test]
fn every_test_file_that_starts_the_binary_isolates_the_machine() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(&directory).expect("tests directory") {
        let path = entry.expect("directory entry").path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if ALLOW.iter().any(|(allowed, _)| *allowed == name) {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("test source");
        let missing = missing_isolation(&source);
        if !missing.is_empty() {
            failures.push(format!(
                "tests/{name} does not mention {}",
                missing.join(", ")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "a test that starts the binary must set scratch paths for it; run scripts/leak-hunt.sh:\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_checker_flags_a_source_that_starts_the_binary_without_the_variables() {
    let bare = r#"Command::new(env!("CARGO_BIN_EXE_greenpiste")).arg("doctor")"#;
    assert_eq!(missing_isolation(bare), REQUIRED.to_vec());
    let partial = format!("{bare} // CLAUDE_CONFIG_DIR");
    assert_eq!(
        missing_isolation(&partial),
        vec!["XDG_STATE_HOME", "GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM"]
    );
    assert!(missing_isolation("fn unrelated() {}").is_empty());
    let system = format!("{partial} XDG_STATE_HOME GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM");
    assert!(
        missing_isolation(&system).is_empty(),
        "GIT_CONFIG_SYSTEM stands in for NOSYSTEM"
    );
}
