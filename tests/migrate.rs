//! `greenpiste migrate` and the preflight every command runs, driven through
//! the real binary on a scratch copy of an old-style machine. Nothing here
//! touches the real `~/.claude`, `~/.local/state` or `~/.gitconfig`: every
//! location the binary reads is pointed into the scratch directory.

use greenpiste_guard::names::{self, old};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

const PLUGIN: &str = "clear-relay";

struct Machine {
    root: PathBuf,
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root); // best-effort cleanup of a temp dir
    }
}

type Snapshot = BTreeMap<String, Vec<u8>>;

impl Machine {
    /// An old-style machine: an old config root with a journal and an outcomes
    /// ledger, an old state root with the plugin marketplace, a settings.json
    /// registering the plugin under the old id, and a global gitconfig with
    /// three old keys.
    fn old_style(name: &str) -> Machine {
        let root =
            std::env::temp_dir().join(format!("gp-migrate-it-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root); // a leftover of a crashed run
        let machine = Machine { root };
        let config_old = machine.claude().join(old::DATA_DIR);
        fs::create_dir_all(config_old.join("plan-review")).unwrap();
        fs::write(
            config_old.join("journal.log"),
            format!("{}\nrow one\n", old::JOURNAL_HEADER),
        )
        .unwrap();
        fs::write(
            config_old.join("plan-review/outcomes.jsonl"),
            format!(
                "{{\"schema\":\"{}\",\"n\":1}}\n",
                old::REVIEW_OUTCOME_SCHEMA
            ),
        )
        .unwrap();
        let old_state = machine.state().join(old::DATA_DIR);
        let marketplace = old_state.join("claude-marketplace");
        fs::create_dir_all(marketplace.join(".claude-plugin")).unwrap();
        write_json(
            &marketplace.join(".claude-plugin/marketplace.json"),
            &json!({ "name": old::DATA_DIR, "plugins": [] }),
        );
        let id = format!("{PLUGIN}@{}", old::DATA_DIR);
        write_json(
            &machine.claude().join("settings.json"),
            &json!({
                "model": "opus",
                "extraKnownMarketplaces": {
                    old::DATA_DIR: { "source": { "source": "directory",
                        "path": marketplace.display().to_string() } }
                },
                "enabledPlugins": { id.clone(): true },
                "pluginConfigs": { id: { "options": {
                    "stateDir": old_state.display().to_string(), "custom": "kept" } } }
            }),
        );
        fs::create_dir_all(machine.home()).unwrap();
        for key in ["enabled", "opener", "git-add-broad.stance"] {
            let status = Command::new("git")
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .args(["config", "--file"])
                .arg(machine.gitconfig())
                .arg(format!("{}{key}", old::CONFIG_PREFIX))
                .arg("on")
                .status()
                .unwrap();
            assert!(status.success());
        }
        machine
    }

    fn claude(&self) -> PathBuf {
        self.root.join("claude")
    }

    fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn gitconfig(&self) -> PathBuf {
        self.home().join(".gitconfig")
    }

    fn done_marker(&self) -> PathBuf {
        self.claude().join(names::MIGRATE_DONE_MARKER)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_greenpiste"));
        command
            .current_dir(&self.root)
            .env("HOME", self.home())
            .env("XDG_CONFIG_HOME", self.root.join("xdg-config"))
            .env("XDG_STATE_HOME", self.state())
            .env("CLAUDE_CONFIG_DIR", self.claude())
            .env("GIT_CONFIG_GLOBAL", self.gitconfig())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GREENPISTE_OFF")
            .env_remove("GREENHARNESS_HOST_SLOT");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("the binary runs")
    }

    fn hook(&self, command_line: &str) -> Output {
        let payload = format!(
            r#"{{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp",
                 "session_id":"sess1234","tool_use_id":"t1","permission_mode":"default",
                 "tool_input":{{"command":{}}}}}"#,
            Value::String(command_line.to_string())
        );
        let mut child = self
            .command()
            .arg("hook")
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
        child.wait_with_output().unwrap()
    }

    /// Everything under the config and state roots and the gitconfig's keys;
    /// a symlink is recorded as its target, never followed.
    fn snapshot(&self) -> Snapshot {
        let mut found = Snapshot::new();
        for base in [self.claude(), self.state()] {
            walk(&base, &base, &mut found);
        }
        found
    }

    fn git_keys(&self) -> Vec<String> {
        let output = Command::new("git")
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["config", "--list", "--file"])
            .arg(self.gitconfig())
            .output()
            .unwrap();
        let mut keys: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect();
        keys.sort();
        keys
    }
}

fn walk(base: &Path, dir: &Path, into: &mut Snapshot) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let key = path.strip_prefix(base).unwrap().display().to_string();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.file_type().is_symlink() {
            let target = fs::read_link(&path).unwrap();
            into.insert(key, format!("-> {}", target.display()).into_bytes());
        } else if meta.is_dir() {
            into.insert(format!("{key}/"), Vec::new());
            walk(base, &path, into);
        } else {
            into.insert(key, fs::read(&path).unwrap());
        }
    }
}

/// The snapshot without the migration's own markers and lock.
fn without_markers(snapshot: &Snapshot) -> Snapshot {
    snapshot
        .iter()
        .filter(|(key, _)| !key.starts_with("greenpiste-migrate"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn write_json(path: &Path, doc: &Value) {
    let mut text = serde_json::to_string_pretty(doc).unwrap();
    text.push('\n');
    fs::write(path, text).unwrap();
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn is_deny(output: &Output) -> bool {
    serde_json::from_slice::<Value>(&output.stdout)
        .ok()
        .and_then(|json| {
            json["hookSpecificOutput"]["permissionDecision"]
                .as_str()
                .map(str::to_string)
        })
        .as_deref()
        == Some("deny")
}

/// A command the guard refuses, so a hook's normal output is visible.
const DENIED: &str = "git push origin main 2>&1 | tail -5";

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

#[test]
fn migrate_moves_everything_and_a_second_run_changes_nothing() {
    let machine = Machine::old_style("forward");
    let first = machine.run(&["migrate"]);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let said = stdout(&first);
    for step in [
        "state-directories",
        "git config keys",
        "plugin registration",
    ] {
        assert!(said.contains(step), "{said}");
    }
    let last = said.lines().last().unwrap();
    assert!(last.starts_with("migrated 3 steps in "), "{said}");

    assert!(machine.claude().join(names::DATA_DIR).is_dir());
    assert!(is_symlink(&machine.claude().join(old::DATA_DIR)));
    assert!(machine.state().join(names::DATA_DIR).is_dir());
    assert!(is_symlink(&machine.state().join(old::DATA_DIR)));
    let settings = fs::read_to_string(machine.claude().join("settings.json")).unwrap();
    assert!(settings.contains(&format!("{PLUGIN}@{}", names::DATA_DIR)));
    assert!(!settings.contains(old::DATA_DIR));
    let marketplace = machine
        .state()
        .join(names::DATA_DIR)
        .join("claude-marketplace/.claude-plugin/marketplace.json");
    assert!(fs::read_to_string(marketplace)
        .unwrap()
        .contains(&format!("\"name\": \"{}\"", names::DATA_DIR)));
    assert!(machine
        .git_keys()
        .iter()
        .all(|line| !line.starts_with(old::CONFIG_PREFIX)));
    assert_eq!(machine.git_keys().len(), 3);

    let after_first = machine.snapshot();
    let second = machine.run(&["migrate"]);
    assert_eq!(second.status.code(), Some(0), "{}", stderr(&second));
    assert_eq!(stdout(&second).trim(), "already migrated");
    assert_eq!(machine.snapshot(), after_first);
}

#[test]
fn reverse_restores_the_original_tree_and_keys() {
    let machine = Machine::old_style("reverse");
    let original = machine.snapshot();
    let original_keys = machine.git_keys();
    assert_eq!(machine.run(&["migrate"]).status.code(), Some(0));
    let reversed = machine.run(&["migrate", "--reverse"]);
    assert_eq!(reversed.status.code(), Some(0), "{}", stderr(&reversed));
    assert_eq!(stdout(&reversed).lines().last(), Some("reversed"));
    assert_eq!(without_markers(&machine.snapshot()), original);
    assert_eq!(machine.git_keys(), original_keys);
    let again = machine.run(&["migrate", "--reverse"]);
    assert_eq!(stdout(&again).trim(), "already reversed");
}

#[test]
fn dry_run_says_what_would_change_and_changes_nothing() {
    let machine = Machine::old_style("dry-run");
    let before = machine.snapshot();
    let keys = machine.git_keys();
    for args in [
        &["migrate", "--dry-run"][..],
        &["migrate", "--dry-run", "--reverse"],
    ] {
        let output = machine.run(args);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert!(stdout(&output)
            .lines()
            .all(|line| line.starts_with("would: ")));
        assert_eq!(machine.snapshot(), before);
        assert_eq!(machine.git_keys(), keys);
    }
    let forward = stdout(&machine.run(&["migrate", "--dry-run"]));
    assert!(
        forward.contains("would: plugin registration: "),
        "{forward}"
    );
    assert!(forward.contains("would: git config keys: "), "{forward}");
    assert!(forward.contains("would: state-directories: "), "{forward}");
}

#[test]
fn a_usage_error_exits_2() {
    let machine = Machine::old_style("usage");
    let output = machine.run(&["migrate", "--bogus"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!machine.done_marker().exists());
}

#[test]
fn a_hook_migrates_once_and_still_decides() {
    let machine = Machine::old_style("hook");
    let first = machine.hook(DENIED);
    assert!(is_deny(&first), "{}", stdout(&first));
    assert_eq!(stderr(&first), "");
    assert!(machine.claude().join(names::DATA_DIR).is_dir());
    let marker = fs::read_to_string(machine.done_marker()).unwrap();
    let millis: u128 = marker
        .trim()
        .parse()
        .expect("the marker holds milliseconds");
    assert!(millis < 60_000, "{marker}");

    let marker_before = fs::read(machine.done_marker()).unwrap();
    let settings_before = fs::read(machine.claude().join("settings.json")).unwrap();
    let keys_before = machine.git_keys();
    let second = machine.hook(DENIED);
    assert!(is_deny(&second));
    assert_eq!(fs::read(machine.done_marker()).unwrap(), marker_before);
    assert_eq!(
        fs::read(machine.claude().join("settings.json")).unwrap(),
        settings_before
    );
    assert_eq!(machine.git_keys(), keys_before);
    assert!(is_symlink(&machine.claude().join(old::DATA_DIR)));
}

#[test]
fn a_hook_never_migrates_after_a_reverse() {
    let machine = Machine::old_style("hook-after-reverse");
    assert_eq!(machine.run(&["migrate"]).status.code(), Some(0));
    assert_eq!(
        machine.run(&["migrate", "--reverse"]).status.code(),
        Some(0)
    );
    let before = without_markers(&machine.snapshot());
    let output = machine.hook(DENIED);
    assert!(is_deny(&output));
    assert!(!machine.done_marker().exists());
    // The hook's own journal may create the new directory; the old one stays put.
    assert!(!is_symlink(&machine.claude().join(old::DATA_DIR)));
    assert!(!machine.state().join(names::DATA_DIR).exists());
    assert!(machine.claude().join(old::DATA_DIR).is_dir());
    assert!(!is_symlink(&machine.claude().join(old::DATA_DIR)));
    let after = without_markers(&machine.snapshot());
    for (key, value) in &before {
        if !key.ends_with(".log") && !key.starts_with(&format!("{}/", old::DATA_DIR)) {
            assert_eq!(after.get(key), Some(value), "{key}");
        }
    }
}

#[test]
fn a_hook_with_the_lock_held_decides_and_moves_nothing() {
    let machine = Machine::old_style("locked");
    fs::create_dir_all(machine.claude()).unwrap();
    let held = greenpiste_guard::lock::lock_within(
        &machine.claude().join(names::MIGRATE_LOCK_FILE),
        Duration::ZERO,
    )
    .unwrap()
    .expect("the test takes the lock");
    let output = machine.hook(DENIED);
    assert!(is_deny(&output), "{}", stdout(&output));
    // The hook's own journal may create the new directory; the old one stays put.
    assert!(!is_symlink(&machine.claude().join(old::DATA_DIR)));
    assert!(!machine.done_marker().exists());
    assert!(machine
        .claude()
        .join(old::DATA_DIR)
        .join("journal.log")
        .is_file());
    drop(held);
}

#[test]
fn a_failing_step_is_attempted_once_by_fifty_hooks() {
    let machine = Machine::old_style("failing");
    let settings_path = machine.claude().join("settings.json");
    let mut settings: Value =
        serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
    settings["enabledPlugins"][format!("{PLUGIN}@{}", names::DATA_DIR)] = json!(true);
    write_json(&settings_path, &settings);
    let failed = machine.claude().join(names::MIGRATE_FAILED_MARKER);

    let first = machine.hook(DENIED);
    assert!(is_deny(&first), "{}", stdout(&first));
    let note = fs::read_to_string(&failed).expect("the first attempt leaves its marker");
    assert!(
        note.contains(&format!("{PLUGIN}@{}", old::DATA_DIR)),
        "{note}"
    );
    let modified = fs::metadata(&failed).unwrap().modified().unwrap();
    for _ in 0..49 {
        let output = machine.hook(DENIED);
        assert!(is_deny(&output), "{}", stdout(&output));
        assert_eq!(stderr(&output), "");
    }
    assert_eq!(fs::read_to_string(&failed).unwrap(), note);
    assert_eq!(fs::metadata(&failed).unwrap().modified().unwrap(), modified);
    assert!(!machine.done_marker().exists());

    let other = machine.run(&["rules"]);
    assert!(
        stderr(&other).contains("migration pending: run greenpiste migrate"),
        "{}",
        stderr(&other)
    );
    let refused = machine.run(&["migrate"]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        stderr(&refused).contains("plugin registration"),
        "{}",
        stderr(&refused)
    );
}

#[test]
fn version_and_help_migrate_nothing() {
    let machine = Machine::old_style("version");
    let before = machine.snapshot();
    for args in [
        &["--version"][..],
        &["--help"],
        &["--bogus", "--help"],
        &["--bogus"],
    ] {
        machine.run(args);
        assert_eq!(machine.snapshot(), before, "{args:?}");
    }
    assert!(!machine.done_marker().exists());
}

/// A step that cannot finish must not switch off what the old keys set: the
/// key sits in a read-only included file, so the config step fails, and the
/// hook still applies its `denyAgents`.
#[cfg(unix)]
#[test]
fn a_failed_step_still_applies_the_old_deny_agents() {
    use std::os::unix::fs::PermissionsExt;
    let machine = Machine::old_style("failed-keeps-old-keys");
    let included = machine.home().join("included.gitconfig");
    let status = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["config", "--file"])
        .arg(&included)
        .arg(format!(
            "{}symbol-grep-not-lsp.denyAgents",
            old::CONFIG_PREFIX
        ))
        .arg("relais:relais-worker-haiku-*")
        .status()
        .unwrap();
    assert!(status.success());
    fs::set_permissions(&included, fs::Permissions::from_mode(0o444)).unwrap();
    let status = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["config", "--file"])
        .arg(machine.gitconfig())
        .args(["include.path"])
        .arg(&included)
        .status()
        .unwrap();
    assert!(status.success());

    fs::create_dir_all(machine.root.join("src")).unwrap();
    let init = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", machine.gitconfig())
        .args(["init", "-q", "--template="])
        .current_dir(&machine.root)
        .status()
        .unwrap();
    assert!(init.success());
    let bin = machine.root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let stub = bin.join("fleet-lsp");
    fs::write(&stub, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        machine.claude().join("settings.json"),
        r#"{"enabledPlugins":{"fleet-lsp@fleet-lsp":true}}"#,
    )
    .unwrap();
    let root = machine.root.join("relais");
    fs::create_dir_all(root.join("agents")).unwrap();
    fs::write(
        root.join("agents/relais-worker-haiku-default.md"),
        "---\nname: w\n---\n",
    )
    .unwrap();
    fs::create_dir_all(machine.claude().join("plugins")).unwrap();
    write_json(
        &machine.claude().join("plugins/installed_plugins.json"),
        &json!({"plugins": {"relais@relais": [{"installPath": root.display().to_string()}]}}),
    );
    let payload = |call: usize| {
        json!({
            "hook_event_name": "PreToolUse", "tool_name": "Grep",
            "cwd": machine.root.display().to_string(), "session_id": format!("sess{call}"),
            "permission_mode": "default",
            "agent_id": "w1", "agent_type": "relais:relais-worker-haiku-default",
            "tool_input": {"pattern": "fn agent_type", "path": "."},
        })
        .to_string()
    };
    let call = |number: usize| {
        let mut child = machine
            .command()
            .arg("hook")
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload(number).as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let failed = machine.claude().join(names::MIGRATE_FAILED_MARKER);
    let first = call(0);
    assert!(
        is_deny(&first),
        "{} / {} / {:?}",
        stdout(&first),
        stderr(&first),
        fs::read_to_string(&failed)
    );
    let note = fs::read_to_string(&failed).expect("the first call attempts and fails");
    let modified = fs::metadata(&failed).unwrap().modified().unwrap();
    for number in 1..50 {
        assert!(is_deny(&call(number)));
    }
    assert_eq!(fs::read_to_string(&failed).unwrap(), note);
    assert_eq!(fs::metadata(&failed).unwrap().modified().unwrap(), modified);
    assert!(!machine.done_marker().exists());
}
