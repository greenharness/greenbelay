//! `greenbelay explain clear-at-boundary --format cases`, end to end through
//! the real binary: the journal's Stops become fixtures and `?` case lines.
//! Every run names `--out`, so nothing is written into the checkout.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};

const SESSION: &str = "feedc0de-0000-4000-8000-000000000000";

struct World {
    root: PathBuf,
}

impl World {
    fn new(name: &str, journal_lines: &[String]) -> World {
        let root = std::env::temp_dir().join(format!(
            "greenbelay-export-bin-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root); // a world left by a killed run, if any
        let projects = root.join("claude/projects/-work");
        let data = root.join("claude/greenbelay");
        for dir in [&projects, &data, &root.join("state")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let entries = [
            json!({"type": "user", "uuid": "p1", "timestamp": "2026-11-18T10:00:01.000Z",
                   "cwd": "/Users/someone/work",
                   "message": {"role": "user", "content": "a person's private words"}}),
            json!({"type": "assistant", "uuid": "b1", "timestamp": "2026-11-18T10:00:02.000Z",
                   "cwd": "/Users/someone/work",
                   "message": {"role": "assistant",
                       "usage": {"input_tokens": 250_000, "cache_creation_input_tokens": 0,
                                 "cache_read_input_tokens": 0},
                       "content": [
                           {"type": "text", "text": "assistant prose"},
                           {"type": "tool_use", "id": "t1", "name": "Bash",
                            "input": {"command": "git worktree remove /Users/someone/wt"}}]}}),
        ];
        let body: String = entries.iter().map(|entry| format!("{entry}\n")).collect();
        std::fs::write(projects.join(format!("{SESSION}.jsonl")), body).unwrap();
        let mut journal = String::from("greenbelay-v1\n");
        for line in journal_lines {
            journal.push_str(line);
            journal.push('\n');
        }
        std::fs::write(data.join("journal.log"), journal).unwrap();
        World { root }
    }

    fn out(&self) -> PathBuf {
        self.root.join("fixtures")
    }

    fn export(&self, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_greenbelay"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", &self.root)
            .args(["explain", "clear-at-boundary", "--format", "cases", "--out"])
            .arg(self.out())
            .args(extra)
            .output()
            .expect("the binary runs")
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.out())
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root); // scratch
    }
}

fn advised(second: u64, session: &str) -> String {
    format!(
        "F {second} clear-at-boundary advise advised {session} work bypassPermissions arm=live worktree remove uuid=b1"
    )
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn stdout_carries_only_case_lines_and_the_summary_comes_first_on_stderr() {
    let world = World::new("contract", &[advised(1_794_960_484, "feedc0de")]);
    let output = world.export(&[]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "?\tStop feedc0de-1794960484\n");
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("wrote 1 fixtures to ")
            && stderr.contains("(0 replay differs, 0 excluded, 0 already exported, 0 skipped)"),
        "{stderr}"
    );
    assert_eq!(
        world.files(),
        ["feedc0de-1794960484.json"],
        "no temp file left"
    );
    let fixture = read(&world.out().join("feedc0de-1794960484.json"));
    assert_eq!(fixture["countable"], json!(true));
    let raw = std::fs::read_to_string(world.out().join("feedc0de-1794960484.json")).unwrap();
    for forbidden in ["/Users/", "assistant prose", "private words"] {
        assert!(!raw.contains(forbidden), "{forbidden} leaked");
    }
}

#[test]
fn a_second_export_writes_nothing_and_exits_0() {
    let world = World::new("again", &[advised(1_794_960_484, "feedc0de")]);
    assert!(world.export(&[]).status.success());
    let before = std::fs::read_to_string(world.out().join("feedc0de-1794960484.json")).unwrap();
    let second = world.export(&[]);
    assert!(second.status.success(), "{}", text(&second.stderr));
    assert!(
        text(&second.stderr).contains("wrote 0 fixtures to ")
            && text(&second.stderr).contains("1 already exported"),
        "{}",
        text(&second.stderr)
    );
    let after = std::fs::read_to_string(world.out().join("feedc0de-1794960484.json")).unwrap();
    assert_eq!(before, after);
    assert_eq!(world.files().len(), 1);
}

#[test]
fn a_conflicting_fixture_is_named_and_skipped_with_exit_1() {
    let world = World::new("conflict", &[advised(1_794_960_484, "feedc0de")]);
    std::fs::create_dir_all(world.out()).unwrap();
    std::fs::write(world.out().join("feedc0de-1794960484.json"), "{}").unwrap();
    let output = world.export(&[]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("exists with other content"));
    assert_eq!(text(&output.stdout), "");
    assert_eq!(
        std::fs::read_to_string(world.out().join("feedc0de-1794960484.json")).unwrap(),
        "{}"
    );
}

#[test]
fn a_stop_with_no_transcript_is_skipped_and_all_skipped_exits_1() {
    let world = World::new("missing", &[advised(1_794_960_484, "00000000")]);
    let output = world.export(&[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("no transcript"));
    assert!(text(&output.stderr).contains("1 skipped"));
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn a_journal_with_no_stop_exits_0() {
    let world = World::new("empty", &[]);
    let output = world.export(&[]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn a_bad_sample_is_a_usage_error() {
    let world = World::new("usage", &[]);
    let output = world.export(&["--sample", "many"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn an_excluded_stop_prints_a_comment_and_no_case_line() {
    let line = "F 1794960484 clear-at-boundary advise advised feedc0de work bypassPermissions arm=live worktree remove uuid=gone".to_string();
    let world = World::new("excluded", &[line]);
    let output = world.export(&[]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(
        stdout.starts_with("# excluded: Stop feedc0de-1794960484: no entry uuid=gone"),
        "{stdout}"
    );
    assert_eq!(stdout.lines().count(), 1);
    assert!(text(&output.stderr).contains("1 excluded"));
    assert_eq!(
        read(&world.out().join("feedc0de-1794960484.json"))["countable"],
        json!(false)
    );
}
