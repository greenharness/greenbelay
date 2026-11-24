//! The owner claims the hooks make: the automatic worktree claim, the
//! observed `relais programme claim`, and the `work-overlap` text that names
//! the owner. Every path is sandboxed: the config dir, the host registry
//! directory and the git config never touch the real `~/.claude`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ALPHA_PID: u32 = 40_001;
const BETA_PID: u32 = 40_002;
const SESSION_A: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const SESSION_B: &str = "bbbbbbbb-0000-4000-8000-000000000002";

struct Scene {
    root: PathBuf,
    primary: PathBuf,
    feature: PathBuf,
    now: u64,
}

impl Scene {
    fn new(name: &str) -> Scene {
        let root =
            std::env::temp_dir().join(format!("gb-owner-hooks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // a leftover of an earlier run of this test
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let primary = root.join("repo");
        std::fs::create_dir_all(primary.join(".git/worktrees/feat")).unwrap();
        std::fs::write(primary.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            primary.join(".git/worktrees/feat/HEAD"),
            "ref: refs/heads/feat/x\n",
        )
        .unwrap();
        std::fs::write(primary.join(".git/worktrees/feat/commondir"), "../..\n").unwrap();
        let feature = root.join("feat");
        std::fs::create_dir_all(&feature).unwrap();
        std::fs::write(
            feature.join(".git"),
            format!(
                "gitdir: {}\n",
                primary.join(".git/worktrees/feat").display()
            ),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        let scene = Scene {
            root,
            primary,
            feature,
            now: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };
        scene.register(ALPHA_PID, SESSION_A, "alpha-name");
        scene.register(BETA_PID, SESSION_B, "beta-name");
        scene
    }

    fn register(&self, pid: u32, session: &str, name: &str) {
        std::fs::write(
            self.root.join("sessions").join(format!("{pid}.json")),
            format!(
                r#"{{"sessionId":"{session}","name":"{name}","pid":{pid},"startedAt":1700000000000,"peerProtocol":1}}"#
            ),
        )
        .unwrap();
    }

    fn global_gitconfig(&self, text: &str) {
        std::fs::write(self.root.join("gitconfig"), text).unwrap();
    }

    fn hook(&self, caller_pid: u32, payload: &str) -> String {
        let mut child = Command::new(env!("CARGO_BIN_EXE_greenbelay"))
            .arg("hook")
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("config"))
            .env("GREENBELAY_HOST_SESSIONS_DIR", self.root.join("sessions"))
            .env("GREENBELAY_TEST_CALLER_PID", caller_pid.to_string())
            .env("GREENBELAY_TEST_NOW", self.now.to_string())
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("XDG_STATE_HOME")
            .env_remove("GREENBELAY_OFF")
            .env_remove("GREENHARNESS_HOST_SLOT")
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
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn write_in(&self, caller_pid: u32, session: &str, dir: &Path) -> String {
        let file = serde_json::Value::String(dir.join("f.txt").display().to_string());
        let cwd = serde_json::Value::String(dir.display().to_string());
        self.hook(
            caller_pid,
            &format!(
                r#"{{"hook_event_name":"PreToolUse","tool_name":"Write","cwd":{cwd},
                     "session_id":"{session}","permission_mode":"default",
                     "tool_input":{{"file_path":{file},"content":"x"}}}}"#
            ),
        )
    }

    fn records(&self, kind: &str) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![self.root.join("config/greenbelay/owners").join(kind)];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|e| e == "json") {
                    found.push(path);
                }
            }
        }
        found
    }

    fn only_record(&self, kind: &str) -> serde_json::Value {
        let records = self.records(kind);
        assert_eq!(records.len(), 1, "{records:?}");
        serde_json::from_str(&std::fs::read_to_string(&records[0]).unwrap()).unwrap()
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root); // scratch only
    }
}

#[test]
fn a_write_in_a_feature_worktree_creates_the_worktree_record() {
    let scene = Scene::new("claim");
    scene.write_in(ALPHA_PID, SESSION_A, &scene.feature);
    let record = scene.only_record("worktree");
    assert_eq!(record["identity"]["session"], SESSION_A);
    assert_eq!(record["identity"]["name"], "alpha-name");
    assert_eq!(record["branch"], "feat/x");
}

#[test]
fn a_write_on_the_primary_checkout_on_main_claims_nothing() {
    let scene = Scene::new("primary");
    scene.write_in(ALPHA_PID, SESSION_A, &scene.primary);
    assert_eq!(scene.records("worktree"), Vec::<PathBuf>::new());
}

#[test]
fn a_second_session_is_told_the_owner_by_name_and_does_not_take_the_record() {
    let scene = Scene::new("overlap");
    scene.write_in(ALPHA_PID, SESSION_A, &scene.feature);
    let said = scene.write_in(BETA_PID, SESSION_B, &scene.feature);
    for part in ["work-overlap", "alpha-name", SESSION_A, "SendMessage"] {
        assert!(said.contains(part), "{part}: {said}");
    }
    assert_eq!(
        scene.only_record("worktree")["identity"]["session"],
        SESSION_A
    );
}

#[test]
fn the_same_process_after_a_clear_takes_the_record_over_on_its_first_write() {
    let scene = Scene::new("successor");
    scene.write_in(ALPHA_PID, SESSION_A, &scene.feature);
    let successor = "cccccccc-0000-4000-8000-000000000003";
    scene.register(ALPHA_PID, successor, "alpha-name");
    scene.write_in(ALPHA_PID, successor, &scene.feature);
    assert_eq!(
        scene.only_record("worktree")["identity"]["session"],
        successor
    );
}

#[test]
fn auto_claim_false_stops_the_automatic_claim() {
    let scene = Scene::new("off");
    scene.global_gitconfig("[greenbelay \"work-owner\"]\n\tauto-claim = false\n");
    scene.write_in(ALPHA_PID, SESSION_A, &scene.feature);
    assert_eq!(scene.records("worktree"), Vec::<PathBuf>::new());
}

#[test]
fn an_observed_programme_claim_writes_the_ticket_record() {
    let scene = Scene::new("ticket");
    let dir = scene.primary.join("docs/programmes/follow-ups");
    let cwd = serde_json::Value::String(scene.feature.display().to_string());
    let command = serde_json::Value::String(format!(
        "relais programme claim follow-ups t02 --dir {}",
        dir.display()
    ));
    scene.hook(
        ALPHA_PID,
        &format!(
            r#"{{"hook_event_name":"PostToolUse","tool_name":"Bash","cwd":{cwd},
                 "session_id":"{SESSION_A}","permission_mode":"default","tool_use_id":"t1",
                 "tool_input":{{"command":{command}}},
                 "tool_response":{{"stdout":"claimed","stderr":"","interrupted":false}}}}"#
        ),
    );
    let record = scene.only_record("ticket");
    assert_eq!(record["identity"]["session"], SESSION_A);
    assert_eq!(record["identity"]["name"], "alpha-name");
    assert!(
        scene
            .records("ticket")
            .iter()
            .any(|path| path.ends_with("repo/follow-ups/t02.json")),
        "{:?}",
        scene.records("ticket")
    );
}
