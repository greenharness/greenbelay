//! `greenbelay migrate` and the preflight every command runs, driven through
//! the real binary on a scratch copy of an old-style machine. Nothing here
//! touches the real `~/.claude`, `~/.local/state` or `~/.gitconfig`: every
//! location the binary reads is pointed into the scratch directory.

use greenbelay_guard::names::{self, old};
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

    /// Claude Code's own registry as it holds the plugin for `marketplace`,
    /// shaped like the real files.
    fn write_registry(&self, marketplace: &str) {
        let plugins = self.claude().join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        let dir = self.state().join(marketplace).join("claude-marketplace");
        write_json(
            &plugins.join("known_marketplaces.json"),
            &json!({ marketplace: { "source": { "source": "directory",
                "path": dir.display().to_string() },
                "installLocation": dir.display().to_string(),
                "lastUpdated": "2026-11-15T10:00:00.000Z" } }),
        );
        write_json(
            &plugins.join("installed_plugins.json"),
            &json!({ "version": 2, "plugins": { format!("{PLUGIN}@{marketplace}"): [{
                "scope": "user", "installPath": dir.display().to_string(),
                "version": "1.0.0" }] } }),
        );
    }

    fn registry_bytes(&self) -> Vec<Vec<u8>> {
        ["known_marketplaces.json", "installed_plugins.json"]
            .iter()
            .map(|name| fs::read(self.claude().join("plugins").join(name)).unwrap_or_default())
            .collect()
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
        let mut command = Command::new(env!("CARGO_BIN_EXE_greenbelay"));
        command
            .current_dir(&self.root)
            .env("HOME", self.home())
            .env("XDG_CONFIG_HOME", self.root.join("xdg-config"))
            .env("XDG_STATE_HOME", self.state())
            .env("CLAUDE_CONFIG_DIR", self.claude())
            .env("GIT_CONFIG_GLOBAL", self.gitconfig())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GREENBELAY_OFF")
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
        [self.claude(), self.state()]
            .iter()
            .flat_map(|base| walk(base, base))
            .collect()
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

fn walk(base: &Path, dir: &Path) -> Snapshot {
    let Ok(entries) = fs::read_dir(dir) else {
        return Snapshot::new();
    };
    let mut found = Snapshot::new();
    for entry in entries {
        let path = entry.unwrap().path();
        let key = path.strip_prefix(base).unwrap().display().to_string();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.file_type().is_symlink() {
            let target = fs::read_link(&path).unwrap();
            found.insert(key, format!("-> {}", target.display()).into_bytes());
        } else if meta.is_dir() {
            found.insert(format!("{key}/"), Vec::new());
            found.extend(walk(base, &path));
        } else {
            found.insert(key, fs::read(&path).unwrap());
        }
    }
    found
}

/// The snapshot without the migration's own markers and lock.
fn without_markers(snapshot: &Snapshot) -> Snapshot {
    snapshot
        .iter()
        .filter(|(key, _)| !key.starts_with("greenbelay-migrate"))
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

/// On unix the old config root is left as a symlink to the new one; off unix no
/// link is made (see `link` in the migration), so there is nothing to check.
fn assert_old_root_links(machine: &Machine) {
    if cfg!(unix) {
        assert!(is_symlink(&machine.claude().join(old::DATA_DIR)));
    }
}

#[test]
fn migrate_moves_everything_and_a_second_run_changes_nothing() {
    let machine = Machine::old_style("forward");
    let first = machine.run(&["migrate"]);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    assert_eq!(stdout(&first), "", "a run's progress is not its answer");
    let said = stderr(&first);
    for step in [
        "state directories: ",
        "git config keys: ",
        "plugin registration: ",
    ] {
        assert!(said.contains(step), "{said}");
    }
    assert!(
        said.lines().any(|l| l.starts_with("migrated 3 steps in ")),
        "{said}"
    );

    assert!(machine.claude().join(names::DATA_DIR).is_dir());
    assert_old_root_links(&machine);
    assert!(machine.state().join(names::DATA_DIR).is_dir());
    if cfg!(unix) {
        assert!(is_symlink(&machine.state().join(old::DATA_DIR)));
    }
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
    assert!(
        stderr(&second).starts_with("nothing to do: already migrated"),
        "{}",
        stderr(&second)
    );
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
    assert!(stderr(&reversed).lines().any(|l| l == "reversed"));
    assert_eq!(without_markers(&machine.snapshot()), original);
    assert_eq!(machine.git_keys(), original_keys);
    let again = machine.run(&["migrate", "--reverse"]);
    assert!(stderr(&again).starts_with("nothing to do: already reversed"));
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
            .all(|line| line.starts_with("would: ")
                || line.starts_with("nothing to do: ")
                || line.starts_with("nothing to move: ")));
        assert_eq!(machine.snapshot(), before);
        assert_eq!(machine.git_keys(), keys);
    }
    let forward = stdout(&machine.run(&["migrate", "--dry-run"]));
    assert!(
        forward.contains("would: plugin registration: "),
        "{forward}"
    );
    assert!(forward.contains("would: git config keys: "), "{forward}");
    assert!(forward.contains("would: state directories: "), "{forward}");
}

#[cfg(unix)]
#[test]
fn dry_run_exits_1_when_the_real_run_would_refuse() {
    use std::os::unix::fs::PermissionsExt;
    let machine = Machine::old_style("dry-run-refuses");
    fs::set_permissions(machine.gitconfig(), fs::Permissions::from_mode(0o444)).unwrap();
    let output = machine.run(&["migrate", "--dry-run"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("would refuse: git config keys: "),
        "{}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("migrate would stop: fix what is named above first"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn dry_run_on_a_machine_with_nothing_to_do_says_so() {
    let machine = Machine::old_style("dry-run-nothing");
    let migrated = machine.run(&["migrate"]);
    assert_eq!(migrated.status.code(), Some(0), "{}", stderr(&migrated));
    machine.write_registry(names::DATA_DIR);
    let forward = machine.run(&["migrate", "--dry-run"]);
    assert_eq!(forward.status.code(), Some(0), "{}", stderr(&forward));
    assert_eq!(stdout(&forward).trim(), "nothing to do: already migrated");
    let reversed = machine.run(&["migrate", "--reverse"]);
    assert_eq!(reversed.status.code(), Some(0), "{}", stderr(&reversed));
    machine.write_registry(old::DATA_DIR);
    let backward = machine.run(&["migrate", "--reverse", "--dry-run"]);
    assert_eq!(backward.status.code(), Some(0), "{}", stderr(&backward));
    assert_eq!(stdout(&backward).trim(), "nothing to do: already reversed");
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
    assert_old_root_links(&machine);
}

/// The journal's lines in `dir`, header included.
fn journal_lines(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("journal.log"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

fn assert_hook_row_in_old_root_only(machine: &Machine) {
    assert!(
        !machine.claude().join(names::DATA_DIR).exists(),
        "a hook that cannot migrate must not create the new root"
    );
    assert!(!is_symlink(&machine.claude().join(old::DATA_DIR)));
    let lines = journal_lines(&machine.claude().join(old::DATA_DIR));
    assert_eq!(lines[1], "row one", "{lines:?}");
    assert!(lines.len() > 2, "the hook's row is missing: {lines:?}");
}

#[test]
fn a_hook_never_migrates_after_a_reverse() {
    let machine = Machine::old_style("hook-after-reverse");
    assert_eq!(machine.run(&["migrate"]).status.code(), Some(0));
    assert_eq!(
        machine.run(&["migrate", "--reverse"]).status.code(),
        Some(0)
    );
    let output = machine.hook(DENIED);
    assert!(is_deny(&output));
    assert!(!machine.done_marker().exists());
    assert!(!machine.state().join(names::DATA_DIR).exists());
    assert_hook_row_in_old_root_only(&machine);
}

#[test]
fn an_explicit_migrate_after_a_hook_following_a_reverse_keeps_every_row() {
    let machine = Machine::old_style("merge-after-reverse");
    assert_eq!(machine.run(&["migrate"]).status.code(), Some(0));
    assert_eq!(
        machine.run(&["migrate", "--reverse"]).status.code(),
        Some(0)
    );
    assert!(is_deny(&machine.hook(DENIED)));
    assert_hook_row_in_old_root_only(&machine);
    let rows_before = journal_lines(&machine.claude().join(old::DATA_DIR)).len() - 1;

    let migrated = machine.run(&["migrate"]);
    assert_eq!(migrated.status.code(), Some(0), "{}", stderr(&migrated));
    for key in machine.snapshot().keys() {
        assert!(!key.contains(".from-old-"), "{key}");
    }
    let lines = journal_lines(&machine.claude().join(names::DATA_DIR));
    let headers = lines
        .iter()
        .filter(|l| *l == names::JOURNAL_HEADER || *l == old::JOURNAL_HEADER)
        .count();
    assert_eq!(headers, 1, "{lines:?}");
    assert!(lines[0] == names::JOURNAL_HEADER || lines[0] == old::JOURNAL_HEADER);
    assert_eq!(lines.len() - 1, rows_before, "{lines:?}");
    assert_eq!(lines[1], "row one");
    assert!(lines[2..].iter().all(|l| l != "row one"));

    let status = machine.run(&["status"]);
    assert_eq!(status.status.code(), Some(0), "{}", stderr(&status));
}

#[test]
fn a_hook_on_a_migrated_machine_writes_into_the_new_root() {
    let machine = Machine::old_style("hook-migrated");
    assert_eq!(machine.run(&["migrate"]).status.code(), Some(0));
    let before = journal_lines(&machine.claude().join(names::DATA_DIR)).len();
    assert!(is_deny(&machine.hook(DENIED)));
    let after = journal_lines(&machine.claude().join(names::DATA_DIR)).len();
    assert!(after > before);
    assert_old_root_links(&machine);
}

#[cfg(unix)] // Windows takes no file lock (holds-until: lock.rs gains one there)
#[test]
fn a_hook_with_the_lock_held_decides_and_moves_nothing() {
    let machine = Machine::old_style("locked");
    fs::create_dir_all(machine.claude()).unwrap();
    let held = greenbelay_guard::lock::lock_within(
        &machine.claude().join(names::MIGRATE_LOCK_FILE),
        Duration::ZERO,
    )
    .unwrap()
    .expect("the test takes the lock");
    let output = machine.hook(DENIED);
    assert!(is_deny(&output), "{}", stdout(&output));
    assert!(!machine.done_marker().exists());
    assert_hook_row_in_old_root_only(&machine);
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
    // The state-directories step runs before the failing one, so the data
    // already lives in the new root; the hooks follow it there.
    let lines = journal_lines(&machine.claude().join(names::DATA_DIR));
    assert_eq!(lines[1], "row one", "{lines:?}");
    assert!(lines.len() > 50, "{lines:?}");
    assert_old_root_links(&machine);

    let other = machine.run(&["rules"]);
    assert!(
        stderr(&other).contains(
            "greenbelay: migration failed (see greenbelay doctor); run greenbelay migrate"
        ),
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

/// Every way of asking a command to only look must leave an old-style
/// machine as it was.
#[test]
fn dry_run_and_check_never_migrate() {
    let machine = Machine::old_style("looks-only");
    let before = machine.snapshot();
    let keys = machine.git_keys();
    let settings = fs::read(machine.claude().join("settings.json")).unwrap();
    for args in [
        &["plan-apply", "--dry-run"][..],
        &["plan-apply", "--check"],
        &["install", "--dry-run"],
        &["uninstall", "--check"],
        &["rules", "--check"],
        &["status", "--help"],
    ] {
        machine.run(args);
        assert_eq!(machine.snapshot(), before, "{args:?}");
        assert_eq!(machine.git_keys(), keys, "{args:?}");
        assert!(!machine.done_marker().exists(), "{args:?}");
    }
    assert_eq!(
        fs::read(machine.claude().join("settings.json")).unwrap(),
        settings
    );
}

#[test]
fn install_migrates_first() {
    let machine = Machine::old_style("install-migrates");
    let output = machine.run(&["install"]);
    assert!(
        !stderr(&output).contains("nothing was installed"),
        "{}",
        stderr(&output)
    );
    assert!(machine.done_marker().exists(), "{}", stderr(&output));
    assert!(machine.claude().join(names::DATA_DIR).is_dir());
    assert!(machine
        .git_keys()
        .iter()
        .all(|line| !line.starts_with(old::CONFIG_PREFIX)));
}

#[cfg(unix)] // Windows takes no file lock (holds-until: lock.rs gains one there)
#[test]
fn install_with_the_lock_held_refuses_and_writes_nothing() {
    let machine = Machine::old_style("install-locked");
    fs::create_dir_all(machine.claude()).unwrap();
    let held = greenbelay_guard::lock::lock_within(
        &machine.claude().join(names::MIGRATE_LOCK_FILE),
        Duration::ZERO,
    )
    .unwrap()
    .expect("the test takes the lock");
    let before = machine.snapshot();
    let output = machine
        .command()
        .args(["install", "--write"])
        .env("GREENBELAY_TEST_MIGRATE_WAIT_MS", "200")
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("migration lock"),
        "{}",
        stderr(&output)
    );
    assert!(!machine.done_marker().exists());
    assert_eq!(machine.snapshot(), before);
    drop(held);
}

fn old_id() -> String {
    format!("{PLUGIN}@{}", old::DATA_DIR)
}

fn new_id() -> String {
    format!("{PLUGIN}@{}", names::DATA_DIR)
}

/// A path as the commands print it: bare when it holds only characters a
/// shell leaves alone, else in single quotes (a Windows path has backslashes).
/// Written out here, not taken from the code under test.
fn quoted(path: std::path::PathBuf) -> String {
    let text = path.display().to_string();
    let plain = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if plain {
        text
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

fn new_dir(machine: &Machine) -> String {
    quoted(
        machine
            .state()
            .join(names::DATA_DIR)
            .join("claude-marketplace"),
    )
}

fn old_dir(machine: &Machine) -> String {
    quoted(
        machine
            .state()
            .join(old::DATA_DIR)
            .join("claude-marketplace"),
    )
}

/// The four commands, forward, in the order to run them.
fn forward_commands(machine: &Machine) -> Vec<String> {
    vec![
        format!("claude plugin marketplace add {}", new_dir(machine)),
        format!("claude plugin install {}", new_id()),
        format!("claude plugin uninstall {}", old_id()),
        format!("claude plugin marketplace remove {}", old::DATA_DIR),
    ]
}

fn reverse_commands(machine: &Machine) -> Vec<String> {
    vec![
        format!("claude plugin marketplace add {}", old_dir(machine)),
        format!("claude plugin install {}", old_id()),
        format!("claude plugin uninstall {}", new_id()),
        format!("claude plugin marketplace remove {}", names::DATA_DIR),
    ]
}

fn assert_in_order(said: &str, commands: &[String]) {
    let mut from = 0;
    for command in commands {
        let at = said[from..]
            .find(command.as_str())
            .unwrap_or_else(|| panic!("{command} not in order in {said}"));
        from += at + command.len();
    }
}

#[test]
fn migrate_names_the_claude_plugin_commands_and_never_writes_the_registry() {
    let machine = Machine::old_style("registry-forward");
    machine.write_registry(old::DATA_DIR);
    let registry = machine.registry_bytes();
    let first = machine.run(&["migrate"]);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    assert_eq!(stdout(&first), "");
    let said = stderr(&first);
    assert!(
        said.contains("Claude Code's plugin registry still lists"),
        "{said}"
    );
    assert_in_order(&said, &forward_commands(&machine));
    assert_eq!(machine.registry_bytes(), registry);

    let second = machine.run(&["migrate"]);
    assert_eq!(second.status.code(), Some(0), "{}", stderr(&second));
    assert!(
        stderr(&second).contains("nothing to do"),
        "{}",
        stderr(&second)
    );
    assert_in_order(&stderr(&second), &forward_commands(&machine));
    assert_eq!(machine.registry_bytes(), registry);

    let reversed = machine.run(&["migrate", "--reverse"]);
    assert_eq!(reversed.status.code(), Some(0), "{}", stderr(&reversed));
    machine.write_registry(names::DATA_DIR);
    let registry = machine.registry_bytes();
    let reversed = machine.run(&["migrate", "--reverse"]);
    assert_eq!(reversed.status.code(), Some(0), "{}", stderr(&reversed));
    assert_in_order(&stderr(&reversed), &reverse_commands(&machine));
    assert_eq!(machine.registry_bytes(), registry);
}

#[test]
fn a_current_registry_adds_nothing_to_migrate() {
    let machine = Machine::old_style("registry-current");
    assert_eq!(machine.run(&["migrate"]).status.code(), Some(0));
    machine.write_registry(names::DATA_DIR);
    let again = machine.run(&["migrate"]);
    assert_eq!(stderr(&again).trim(), "nothing to do: already migrated");
}

#[test]
fn an_unreadable_registry_file_is_said_without_commands() {
    let machine = Machine::old_style("registry-unreadable");
    machine.write_registry(old::DATA_DIR);
    let file = machine.claude().join("plugins/installed_plugins.json");
    fs::remove_file(&file).unwrap();
    fs::create_dir(&file).unwrap();
    let output = machine.run(&["migrate"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stderr(&output);
    let path = machine
        .claude()
        .join("plugins")
        .join("installed_plugins.json");
    assert!(
        said.contains(&format!(
            "could not read Claude Code's plugin registry {}: ",
            path.display()
        )),
        "{said}"
    );
    assert!(
        said.contains("  check the file, or run claude plugin list"),
        "{said}"
    );
    assert!(!said.contains("claude plugin install"), "{said}");
}

fn would_then_need(commands: &[String]) -> Vec<String> {
    commands
        .iter()
        .map(|c| format!("would then need: {c}"))
        .collect()
}

#[test]
fn dry_run_says_the_intended_registry_commands_as_would_then_need_lines() {
    let machine = Machine::old_style("registry-dry");
    machine.write_registry(old::DATA_DIR);
    let registry = machine.registry_bytes();
    let forward = machine.run(&["migrate", "--dry-run"]);
    assert_eq!(forward.status.code(), Some(0), "{}", stderr(&forward));
    assert_in_order(
        &stderr(&forward),
        &would_then_need(&forward_commands(&machine)),
    );
    // stdout is the migration's own plan; the commands are the person's.
    assert!(
        !stdout(&forward).contains("claude plugin"),
        "{}",
        stdout(&forward)
    );
    assert_eq!(machine.registry_bytes(), registry);

    assert_eq!(machine.run(&["migrate"]).status.code(), Some(0));
    let already = machine.run(&["migrate", "--dry-run"]);
    assert_eq!(stdout(&already).trim(), "nothing to move: already migrated");
    assert_in_order(
        &stderr(&already),
        &would_then_need(&forward_commands(&machine)),
    );

    machine.write_registry(names::DATA_DIR);
    let registry = machine.registry_bytes();
    let backward = machine.run(&["migrate", "--dry-run", "--reverse"]);
    assert_eq!(backward.status.code(), Some(0), "{}", stderr(&backward));
    assert_in_order(
        &stderr(&backward),
        &would_then_need(&reverse_commands(&machine)),
    );
    assert_eq!(machine.registry_bytes(), registry);
}

#[test]
fn a_dry_run_names_no_commands_when_settings_registers_no_plugin() {
    let machine = Machine::old_style("registry-dry-unused");
    machine.write_registry(old::DATA_DIR);
    fs::write(machine.claude().join("settings.json"), "{}").unwrap();
    let dry = machine.run(&["migrate", "--dry-run"]);
    let real = machine.run(&["migrate"]);
    for output in [&dry, &real] {
        assert_eq!(output.status.code(), Some(0), "{}", stderr(output));
        assert!(
            !stderr(output).contains("claude plugin"),
            "{}",
            stderr(output)
        );
        assert!(
            !stdout(output).contains("claude plugin"),
            "{}",
            stdout(output)
        );
    }
}

#[test]
fn a_hook_run_prints_no_registry_lines() {
    let machine = Machine::old_style("registry-hook");
    machine.write_registry(old::DATA_DIR);
    let output = machine.hook(DENIED);
    assert!(
        !stderr(&output).contains("claude plugin"),
        "{}",
        stderr(&output)
    );
    assert!(
        !stdout(&output).contains("claude plugin"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn an_install_preview_names_the_commands_the_write_then_prints() {
    let machine = Machine::old_style("registry-install");
    assert_eq!(machine.run(&["migrate"]).status.code(), Some(0));
    machine.write_registry(old::DATA_DIR);
    let registry = machine.registry_bytes();
    let wanted = forward_commands(&machine);
    for extra in [&[][..], &["--project"], &["--local"]] {
        let mut args = vec!["install"];
        args.extend_from_slice(extra);
        let preview = machine.run(&args);
        assert_eq!(preview.status.code(), Some(0), "{}", stderr(&preview));
        assert_in_order(&stderr(&preview), &would_then_need(&wanted));
        args.push("--write");
        // Twice: the second finds the plugin files already placed, and still
        // names the commands its preview named.
        for _ in 0..2 {
            let output = machine.run(&args);
            assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
            let said = stderr(&output);
            assert!(said.contains("; run:"), "{said}");
            assert_in_order(&said, &wanted);
        }
    }
    assert_eq!(machine.registry_bytes(), registry);
}

#[test]
fn an_install_without_a_home_prints_no_registry_lines() {
    let machine = Machine::old_style("registry-nohome");
    let output = machine
        .command()
        .arg("install")
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .unwrap();
    assert!(
        !stderr(&output).contains("claude plugin"),
        "{}",
        stderr(&output)
    );
    assert!(
        !stdout(&output).contains("claude plugin"),
        "{}",
        stdout(&output)
    );
}
