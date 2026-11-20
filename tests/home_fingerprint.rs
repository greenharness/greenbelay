//! `home-fingerprint-drift`, driven through the real binary: a PreToolUse
//! keeps the fingerprint, the PostToolUse (or PostToolUseFailure) of the same
//! `tool_use_id` compares it. `HOME`, `CLAUDE_CONFIG_DIR` and
//! `XDG_CONFIG_HOME` are scratch directories, so nothing here reads or
//! writes the developer's own files.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
    xdg: PathBuf,
    repo: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "gb-homefp-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root); // absent on a fresh run
        let home = root.join("home");
        let config = root.join("claude");
        let xdg = root.join("xdg");
        let repo = root.join("repo");
        for d in [&home, &config, &xdg, &repo] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(home.join(".gitconfig"), "[user]\n\tname = t\n").unwrap();
        std::fs::create_dir_all(xdg.join("git")).unwrap();
        std::fs::write(xdg.join("git").join("config"), "[core]\n").unwrap();
        let ok = Command::new("git")
            .args(["init", "-q", "--template="])
            .arg(&repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git init");
        Fixture {
            root,
            home,
            config,
            xdg,
            repo,
        }
    }

    fn send(&self, payload: &str) -> String {
        let mut child = Command::new(env!("CARGO_BIN_EXE_greenbelay"))
            .arg("hook")
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CLAUDE_CONFIG_DIR", &self.config)
            .env("XDG_CONFIG_HOME", &self.xdg)
            .env_remove("XDG_STATE_HOME")
            .env("GIT_CONFIG_GLOBAL", self.root.join("no-global"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GREENBELAY_OFF")
            // Inside a host slot, as under `slot run`: `heavy-build-unqueued`
            // would otherwise advise on every PreToolUse of a test run here.
            .env("GREENHARNESS_HOST_SLOT", "held")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the binary runs");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(0));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn event(&self, event: &str, id: &str, command: &str) -> String {
        let extra = match event {
            "PreToolUse" => String::new(),
            "PostToolUseFailure" => r#","error":"Exit code 101""#.to_string(),
            _ => r#","tool_response":{"stdout":"","stderr":"","interrupted":false}"#.to_string(),
        };
        self.send(&format!(
            r#"{{"hook_event_name":"{event}","tool_name":"Bash","cwd":{},
                 "session_id":"sess1234","tool_use_id":"{id}","permission_mode":"default",
                 "tool_input":{{"command":{}}}{extra}}}"#,
            serde_json::Value::String(self.repo.display().to_string()),
            serde_json::Value::String(command.to_string())
        ))
    }

    fn git_config_local(&self) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args(["config", "--local", "x.y", "1"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git config --local");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root); // best-effort: temp is cleaned eventually
    }
}

fn context(stdout: &str) -> String {
    serde_json::from_str::<serde_json::Value>(stdout)
        .ok()
        .and_then(|v| {
            Some(
                v.get("hookSpecificOutput")?
                    .get("additionalContext")?
                    .as_str()?
                    .to_string(),
            )
        })
        .unwrap_or_default()
}

fn ends_with_path(text: &str, tail: &Path, change: &str) -> bool {
    let tail = tail.display().to_string();
    text.lines()
        .any(|l| l.trim_start().ends_with(&format!("{tail} ({change})")))
}

#[test]
fn a_test_run_that_writes_the_repo_config_is_named() {
    let fixture = Fixture::new("repo-config");
    assert_eq!(fixture.event("PreToolUse", "hf1", "cargo test"), "");
    fixture.git_config_local();
    let said = context(&fixture.event("PostToolUse", "hf1", "cargo test"));
    assert!(said.contains("home-fingerprint-drift"), "{said}");
    assert!(
        ends_with_path(&said, &Path::new(".git").join("config"), "modified"),
        "{said}"
    );
    assert!(said.contains("at the same time"), "{said}");
}

#[test]
fn a_test_run_that_writes_nothing_is_silent() {
    let fixture = Fixture::new("quiet");
    assert_eq!(fixture.event("PreToolUse", "hf2", "cargo test"), "");
    assert_eq!(fixture.event("PostToolUse", "hf2", "cargo test"), "");
}

/// The 2026-11-15 leak rewrote `~/.config/git/config`: replayed under a
/// scratch `XDG_CONFIG_HOME`, it warns once, on a red run too, and a second
/// PostToolUse of the same id has no snapshot left to compare.
#[test]
fn the_11_15_rewrite_of_the_xdg_git_config_warns_once() {
    let fixture = Fixture::new("xdg");
    assert_eq!(
        fixture.event("PreToolUse", "hf3", "cargo test --workspace"),
        ""
    );
    std::fs::write(
        fixture.xdg.join("git").join("config"),
        "[core]\n\thooksPath = x\n",
    )
    .unwrap();
    let said = context(&fixture.event("PostToolUseFailure", "hf3", "cargo test --workspace"));
    assert!(
        ends_with_path(&said, &Path::new("git").join("config"), "modified"),
        "{said}"
    );
    assert_eq!(
        fixture.event("PostToolUseFailure", "hf3", "cargo test --workspace"),
        ""
    );
}

/// The store is first shown to fill for a test run, so an empty count for
/// `git status` cannot come from looking in the wrong directory.
#[test]
fn a_command_that_is_not_a_test_run_keeps_no_snapshot() {
    let fixture = Fixture::new("other");
    let store = fixture
        .config
        .join("greenbelay")
        .join("previews")
        .join("home");
    let kept = || {
        std::fs::read_dir(&store)
            .map(|dir| dir.count())
            .unwrap_or(0)
    };
    assert_eq!(fixture.event("PreToolUse", "hf4a", "cargo test"), "");
    assert_eq!(kept(), 1, "{}", store.display());
    assert_eq!(fixture.event("PreToolUse", "hf4b", "git status"), "");
    assert_eq!(kept(), 1, "{}", store.display());
    fixture.git_config_local();
    assert_eq!(fixture.event("PostToolUse", "hf4b", "git status"), "");
}

#[test]
fn a_new_ssh_file_under_home_is_named() {
    let fixture = Fixture::new("ssh");
    assert_eq!(
        fixture.event("PreToolUse", "hf5", "./target/debug/tool migrate"),
        ""
    );
    std::fs::create_dir_all(fixture.home.join(".ssh")).unwrap();
    std::fs::write(fixture.home.join(".ssh").join("config"), "Host *\n").unwrap();
    std::fs::write(fixture.home.join(".ssh").join("known_hosts"), "h\n").unwrap();
    let said = context(&fixture.event("PostToolUse", "hf5", "./target/debug/tool migrate"));
    assert!(
        ends_with_path(&said, &Path::new(".ssh").join("config"), "added"),
        "{said}"
    );
    assert!(!said.contains("known_hosts"), "{said}");
}
