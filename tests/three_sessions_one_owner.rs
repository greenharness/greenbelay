//! The f107 replay (decisions, 2026-11-23): three sessions on one task, and
//! a fourth process landing a push. A owns a feature worktree (its first
//! write) and the ticket (an observed `relais programme claim`), then clears.
//! The clear-relay starts C on `resume … f107-…`. B pushes from another
//! worktree. Every path is sandboxed: a scratch git repository, a fake host
//! registry, a private config dir.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const TICKET: &str = "f107-application-landscape-theme-onto-override-api";
const A_PID: u32 = 50_001;
const B_PID: u32 = 50_002;
const C_PID: u32 = 50_003;
const A: &str = "aaaaaaaa-1111-4000-8000-000000000001";
const A_AFTER: &str = "aaaaaaaa-2222-4000-8000-000000000002";
const B: &str = "bbbbbbbb-1111-4000-8000-000000000003";
const C: &str = "cccccccc-1111-4000-8000-000000000004";
const A_NAME: &str = "theme-tokens-per-mode-css";

struct Replay {
    root: PathBuf,
    primary: PathBuf,
    feature: PathBuf,
    other: PathBuf,
}

fn git(dir: &Path, args: &[&str]) -> (String, String) {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

impl Replay {
    fn new(name: &str) -> Replay {
        let root = std::env::temp_dir().join(format!("gb-f107-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // a leftover of an earlier run of this test
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let origin = root.join("origin.git");
        let primary = root.join("decisions");
        std::fs::create_dir_all(&origin).unwrap();
        std::fs::create_dir_all(primary.join("docs/programmes/follow-ups")).unwrap();
        git(&origin, &["init", "-q", "--bare"]);
        git(&primary, &["init", "-q"]);
        git(
            &primary,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git(&primary, &["commit", "-q", "--allow-empty", "-m", "one"]);
        let feature = root.join("decisions-wt-theme");
        let other = root.join("decisions-wt-b");
        let add = |path: &Path, branch: &str| {
            git(
                &primary,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    branch,
                    path.to_str().unwrap(),
                ],
            );
        };
        add(&feature, "feat/theme-on-override-api");
        add(&other, "feat/landscape-tokens");
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        let replay = Replay {
            root,
            primary,
            feature,
            other,
        };
        replay.register(A_PID, A, A_NAME);
        replay.register(B_PID, B, "perso-b");
        replay.register(C_PID, C, "perso-c");
        replay
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

    fn command(&self, caller: u32, cwd: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_greenbelay"));
        command
            .current_dir(cwd)
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("config"))
            .env("GREENBELAY_HOST_SESSIONS_DIR", self.root.join("sessions"))
            .env("GREENBELAY_TEST_CALLER_PID", caller.to_string())
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("XDG_STATE_HOME")
            .env_remove("GREENBELAY_OFF")
            .env_remove("GREENHARNESS_HOST_SLOT");
        command
    }

    fn hook(&self, caller: u32, payload: &serde_json::Value) -> String {
        let mut child = self
            .command(caller, &self.root)
            .arg("hook")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn run(&self, caller: u32, cwd: &Path, args: &[&str]) -> (i32, String, String) {
        let out = self.command(caller, cwd).args(args).output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// A Write by `session` in `dir`.
    fn write_in(&self, caller: u32, session: &str, dir: &Path) {
        self.hook(
            caller,
            &serde_json::json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Write",
                "cwd": dir,
                "session_id": session,
                "permission_mode": "default",
                "tool_input": {"file_path": dir.join("notes.md"), "content": "x"},
            }),
        );
    }

    /// A Bash command by `session` in `cwd` that exited 0.
    fn bash_done(&self, caller: u32, session: &str, cwd: &Path, command: &str, stderr: &str) {
        self.hook(
            caller,
            &serde_json::json!({
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "cwd": cwd,
                "session_id": session,
                "permission_mode": "default",
                "tool_use_id": "t",
                "tool_input": {"command": command},
                "tool_response": {"stdout": "", "stderr": stderr, "interrupted": false},
            }),
        );
    }

    /// The prompt hook's context for `text`, or `None` when it is silent.
    fn resume(&self, caller: u32, session: &str) -> Option<String> {
        let out = self.hook(
            caller,
            &serde_json::json!({
                "hook_event_name": "UserPromptSubmit",
                "prompt": format!("resume decisions follow-ups {TICKET}: theme onto the override API"),
                "cwd": self.primary,
                "session_id": session,
                "prompt_id": "p",
            }),
        );
        if out.trim().is_empty() {
            return None;
        }
        let reply: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        Some(
            reply["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .to_owned(),
        )
    }

    /// A owns the feature worktree by its first write, with a ref it reports
    /// itself, and the ticket by an observed `relais programme claim`.
    fn a_starts(&self, with_ticket: bool) {
        self.write_in(A_PID, A, &self.feature);
        let (code, _, err) = self.run(A_PID, &self.feature, &["owner", "claim", "--ref", "417dc6"]);
        assert_eq!(code, 0, "{err}");
        if with_ticket {
            let dir = self.primary.join("docs/programmes/follow-ups");
            self.bash_done(
                A_PID,
                A,
                &self.feature,
                &format!(
                    "relais programme claim follow-ups {TICKET} --dir {}",
                    dir.display()
                ),
                "",
            );
        }
    }

    /// A's process clears: same pid and start, a new session.
    fn a_clears(&self) {
        self.register(A_PID, A_AFTER, A_NAME);
    }

    /// B publishes its branch from its own worktree.
    fn b_pushes(&self) {
        let (_, printed) = git(&self.other, &["push", "origin", "feat/landscape-tokens"]);
        self.bash_done(
            B_PID,
            B,
            &self.other,
            "git push origin feat/landscape-tokens",
            &printed,
        );
    }

    fn owner_of(&self, worktree: &Path) -> serde_json::Value {
        let (code, out, err) = self.run(
            C_PID,
            &self.root,
            &[
                "owner",
                "show",
                "--worktree",
                worktree.to_str().unwrap(),
                "--json",
            ],
        );
        assert_eq!(code, 0, "{err}");
        serde_json::from_str(out.trim()).unwrap()
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root); // scratch only
    }
}

#[test]
fn order_one_c_is_told_a_cleared_and_b_is_credited_with_its_push() {
    let replay = Replay::new("one");
    replay.a_starts(true);
    replay.a_clears();
    let told = replay
        .resume(C_PID, C)
        .expect("C is told who owns the ticket");
    for part in [
        format!("{A} held {TICKET}"),
        format!("its process {A_NAME} has cleared"),
        format!("now runs {A_AFTER}"),
        "do not credit it".to_owned(),
    ] {
        assert!(told.contains(&part), "{part}: {told}");
    }
    replay.b_pushes();
    let (code, out, _) = replay.run(
        C_PID,
        &replay.root,
        &["who-pushed", "feat/landscape-tokens"],
    );
    assert_eq!(code, 0);
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(out.contains("perso-b") && out.contains(&B[..8]), "{out}");
    assert!(!out.contains("perso-c") && !out.contains(&C[..8]), "{out}");
    // A′'s first write takes the worktree over; the board shows one owner,
    // A′, by name and UUID, with A's self-reported ref dropped.
    replay.write_in(A_PID, A_AFTER, &replay.feature);
    let (code, json, err) = replay.run(C_PID, &replay.primary, &["board", "--json"]);
    assert_eq!(code, 0, "{err}");
    let board: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    let trees = board[0]["worktrees"].as_array().unwrap();
    let feature = trees
        .iter()
        .find(|tree| tree["branch"] == "feat/theme-on-override-api")
        .unwrap();
    assert_eq!(feature["owner"]["session"], A_AFTER);
    assert_eq!(feature["owner"]["name"], A_NAME);
    assert_eq!(feature["owner"]["state"], "live");
    assert!(feature["owner"]["ref"].is_null(), "{feature}");
    let owners: Vec<&serde_json::Value> = trees
        .iter()
        .filter_map(|tree| tree["owner"].as_object().map(|_| &tree["owner"]))
        .filter(|owner| owner["session"] == A || owner["session"] == A_AFTER)
        .collect();
    assert_eq!(owners.len(), 1, "{trees:?}");
    let pushed = trees
        .iter()
        .find(|tree| tree["branch"] == "feat/landscape-tokens")
        .unwrap();
    assert_eq!(pushed["pushed"]["session"], B);
    let (code, _, err) = replay.run(
        A_PID,
        &replay.feature,
        &["owner", "claim", "--ref", "9f00aa"],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(replay.owner_of(&replay.feature)["ref"], "9f00aa");
}

#[test]
fn order_two_a_after_resumes_first_and_c_is_told_the_owner_is_live() {
    let replay = Replay::new("two");
    replay.a_starts(true);
    replay.a_clears();
    let continued = replay
        .resume(A_PID, A_AFTER)
        .expect("A′ is told it continues");
    assert!(
        continued.contains(&format!(
            "you continue {TICKET}, held by this process as {A}"
        )),
        "{continued}"
    );
    let told = replay
        .resume(C_PID, C)
        .expect("C is told who owns the ticket");
    for part in [
        format!("{TICKET} is owned by {A_NAME}"),
        A_AFTER.to_owned(),
        format!("SendMessage to \"{A_NAME}\""),
    ] {
        assert!(told.contains(&part), "{part}: {told}");
    }
    assert!(!told.contains("cleared"), "{told}");
}

/// The documented limit: with no ticket record, a worktree on a feature
/// branch says nothing about the ticket, so C is not told.
#[test]
fn without_a_ticket_record_a_feature_branch_owner_is_not_found() {
    let replay = Replay::new("no-ticket");
    replay.a_starts(false);
    assert_eq!(replay.resume(C_PID, C), None);
}
