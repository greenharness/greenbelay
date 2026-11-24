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

    /// A linked worktree of `repo` (created when missing) on `branch`.
    fn linked(&self, repo: &str, name: &str, branch: &str) -> PathBuf {
        let primary = self.root.join(repo);
        let admin = primary.join(".git/worktrees").join(name);
        std::fs::create_dir_all(&admin).unwrap();
        if !primary.join(".git/HEAD").exists() {
            std::fs::write(primary.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        }
        std::fs::write(admin.join("HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();
        std::fs::write(admin.join("commondir"), "../..\n").unwrap();
        let tree = self.root.join(format!("{repo}-{name}"));
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        tree
    }

    /// An observed `relais programme claim follow-ups t02 --dir …` by
    /// `session` in process `pid`: the ticket record `repo/follow-ups/t02`.
    fn claim_ticket(&self, pid: u32, session: &str) {
        let dir = self.primary.join("docs/programmes/follow-ups");
        let cwd = serde_json::Value::String(self.feature.display().to_string());
        let command = serde_json::Value::String(format!(
            "relais programme claim follow-ups t02 --dir {}",
            dir.display()
        ));
        self.hook(
            pid,
            &format!(
                r#"{{"hook_event_name":"PostToolUse","tool_name":"Bash","cwd":{cwd},
                     "session_id":"{session}","permission_mode":"default","tool_use_id":"t1",
                     "tool_input":{{"command":{command}}},
                     "tool_response":{{"stdout":"claimed","stderr":"","interrupted":false}}}}"#
            ),
        );
    }

    /// The prompt hook's reply, parsed; `Null` when it printed nothing.
    fn prompt(&self, caller_pid: u32, session: &str, text: &str) -> serde_json::Value {
        let cwd = serde_json::Value::String(self.primary.display().to_string());
        let text = serde_json::Value::String(text.to_owned());
        let out = self.hook(
            caller_pid,
            &format!(
                r#"{{"hook_event_name":"UserPromptSubmit","prompt":{text},"cwd":{cwd},
                     "session_id":"{session}","prompt_id":"p1"}}"#
            ),
        );
        if out.trim().is_empty() {
            return serde_json::Value::Null;
        }
        assert_eq!(out.trim().lines().count(), 1, "one JSON object: {out}");
        serde_json::from_str(out.trim()).unwrap()
    }

    /// `greenbelay <args>` in this scene's sandbox: (exit code, stdout, stderr).
    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_greenbelay"))
            .args(args)
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("config"))
            .env("GREENBELAY_HOST_SESSIONS_DIR", self.root.join("sessions"))
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("XDG_STATE_HOME")
            .env_remove("GREENBELAY_OFF")
            .output()
            .expect("the binary runs");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// A PostToolUse for a Bash `command` that exited 0 in `cwd`, with the
    /// output it printed.
    fn bash_done(
        &self,
        pid: u32,
        session: &str,
        cwd: &Path,
        command: &str,
        stdout: &str,
        stderr: &str,
    ) {
        let cwd = serde_json::Value::String(cwd.display().to_string());
        let command = serde_json::Value::String(command.to_owned());
        let stdout = serde_json::Value::String(stdout.to_owned());
        let stderr = serde_json::Value::String(stderr.to_owned());
        self.hook(
            pid,
            &format!(
                r#"{{"hook_event_name":"PostToolUse","tool_name":"Bash","cwd":{cwd},
                     "session_id":"{session}","permission_mode":"default","tool_use_id":"tp",
                     "tool_input":{{"command":{command}}},
                     "tool_response":{{"stdout":{stdout},"stderr":{stderr},"interrupted":false}}}}"#
            ),
        );
    }

    fn pushes(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("config/greenbelay/pushes.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
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

const RESUME: &str = "resume repo follow-ups t02: Three sessions on one task";
const SESSION_C: &str = "cccccccc-0000-4000-8000-000000000003";
const GAMMA_PID: u32 = 40_003;

fn context(reply: &serde_json::Value) -> String {
    assert_eq!(
        reply["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit",
        "{reply}"
    );
    reply["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn a_resume_of_a_ticket_a_live_session_owns_names_the_owner_and_warns_the_person() {
    let scene = Scene::new("resume-live");
    scene.claim_ticket(ALPHA_PID, SESSION_A);
    scene.register(GAMMA_PID, SESSION_C, "gamma-name");
    let reply = scene.prompt(GAMMA_PID, SESSION_C, RESUME);
    let said = context(&reply);
    for part in [
        "work-owner",
        "t02 is owned by alpha-name",
        SESSION_A,
        "SendMessage to \"alpha-name\"",
    ] {
        assert!(said.contains(part), "{part}: {said}");
    }
    let note = reply["systemMessage"].as_str().unwrap();
    assert!(note.starts_with("WARNING:"), "{note}");
    assert!(note.lines().count() <= 2, "{note}");
    assert_eq!(
        scene.only_record("ticket")["identity"]["session"],
        SESSION_A
    );
}

#[test]
fn the_owners_own_process_after_a_clear_continues_and_takes_the_ticket_over() {
    let scene = Scene::new("resume-successor");
    scene.claim_ticket(ALPHA_PID, SESSION_A);
    let successor = "dddddddd-0000-4000-8000-000000000004";
    // The registry has not caught up: ALPHA's file still names SESSION_A.
    let said = context(&scene.prompt(ALPHA_PID, successor, RESUME));
    assert!(
        said.contains(&format!(
            "you continue t02, held by this process as {SESSION_A} before /clear"
        )),
        "{said}"
    );
    assert_eq!(
        scene.only_record("ticket")["identity"]["session"],
        successor
    );
}

#[test]
fn an_owner_whose_process_cleared_is_named_and_not_credited() {
    let scene = Scene::new("resume-cleared");
    scene.claim_ticket(ALPHA_PID, SESSION_A);
    let successor = "dddddddd-0000-4000-8000-000000000004";
    scene.register(ALPHA_PID, successor, "alpha-name");
    scene.register(GAMMA_PID, SESSION_C, "gamma-name");
    let reply = scene.prompt(GAMMA_PID, SESSION_C, RESUME);
    let said = context(&reply);
    for part in [
        format!("{SESSION_A} held t02"),
        "its process alpha-name has cleared".to_owned(),
        format!("now runs {successor}"),
        "do not credit it".to_owned(),
    ] {
        assert!(said.contains(&part), "{part}: {said}");
    }
    assert!(reply["systemMessage"]
        .as_str()
        .unwrap()
        .starts_with("WARNING:"));
    assert_eq!(
        scene.only_record("ticket")["identity"]["session"],
        SESSION_A
    );
}

#[test]
fn without_a_ticket_record_the_worktree_on_the_prog_branch_of_that_repository_answers() {
    let scene = Scene::new("resume-fallback");
    let tree = scene.linked("repo", "t02", "prog/follow-ups/t02");
    scene.write_in(ALPHA_PID, SESSION_A, &tree);
    scene.register(GAMMA_PID, SESSION_C, "gamma-name");
    let said = context(&scene.prompt(GAMMA_PID, SESSION_C, RESUME));
    assert!(said.contains("t02 is owned by alpha-name"), "{said}");
    assert!(
        said.contains(&format!("worktree {}", tree.display())),
        "{said}"
    );
}

#[test]
fn the_same_branch_in_another_repository_is_not_this_tickets_owner() {
    let scene = Scene::new("resume-other-repo");
    let tree = scene.linked("other", "t02", "prog/follow-ups/t02");
    scene.write_in(ALPHA_PID, SESSION_A, &tree);
    scene.register(GAMMA_PID, SESSION_C, "gamma-name");
    assert_eq!(
        scene.prompt(GAMMA_PID, SESSION_C, RESUME),
        serde_json::Value::Null
    );
}

#[test]
fn records_that_disagree_are_all_named_and_nothing_is_taken_over() {
    let scene = Scene::new("resume-disagree");
    scene.claim_ticket(ALPHA_PID, SESSION_A);
    let tree = scene.linked("repo", "t02", "prog/follow-ups/t02");
    scene.write_in(BETA_PID, SESSION_B, &tree);
    let successor = "dddddddd-0000-4000-8000-000000000004";
    let reply = scene.prompt(ALPHA_PID, successor, RESUME);
    let said = context(&reply);
    for part in ["disagree", SESSION_A, SESSION_B, "take nothing over"] {
        assert!(said.contains(part), "{part}: {said}");
    }
    assert!(reply["systemMessage"]
        .as_str()
        .unwrap()
        .starts_with("WARNING:"));
    assert_eq!(
        scene.only_record("ticket")["identity"]["session"],
        SESSION_A
    );
}

#[test]
fn no_registered_caller_is_unknown_never_owned_by() {
    let scene = Scene::new("resume-no-caller");
    scene.claim_ticket(ALPHA_PID, SESSION_A);
    let said = context(&scene.prompt(49_999, SESSION_C, RESUME));
    assert!(said.contains("ownership of t02 is unknown"), "{said}");
    assert!(!said.contains("owned by"), "{said}");
}

#[test]
fn a_driver_prompt_and_an_unowned_ticket_are_silent() {
    let scene = Scene::new("resume-silent");
    scene.register(GAMMA_PID, SESSION_C, "gamma-name");
    assert_eq!(
        scene.prompt(GAMMA_PID, SESSION_C, RESUME),
        serde_json::Value::Null
    );
    scene.claim_ticket(ALPHA_PID, SESSION_A);
    assert_eq!(
        scene.prompt(
            GAMMA_PID,
            SESSION_C,
            "resume repo follow-ups drive: t02 t03"
        ),
        serde_json::Value::Null
    );
}

/// `git <args>` in `dir`, which must succeed: (stdout, stderr).
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

/// A real repository `work` on `feat/x` with one commit, and a bare `origin`.
fn real_repo(scene: &Scene) -> PathBuf {
    let origin = scene.root.join("origin.git");
    let work = scene.root.join("work");
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    git(&origin, &["init", "-q", "--bare"]);
    git(&work, &["init", "-q"]);
    git(
        &work,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&work, &["commit", "-q", "--allow-empty", "-m", "one"]);
    git(&work, &["switch", "-q", "-c", "feat/x"]);
    work
}

#[test]
fn a_push_is_credited_to_its_session_and_an_unchanged_repeat_to_no_one() {
    let scene = Scene::new("push");
    let work = real_repo(&scene);
    let (_, created) = git(&work, &["push", "origin", "feat/x"]);
    let first = git(&work, &["rev-parse", "HEAD"]).0.trim().to_owned();
    scene.bash_done(
        BETA_PID,
        SESSION_B,
        &work,
        "git push origin feat/x",
        "",
        &created,
    );
    git(&work, &["commit", "-q", "--allow-empty", "-m", "two"]);
    let (_, updated) = git(&work, &["push", "origin", "feat/x"]);
    let second = git(&work, &["rev-parse", "HEAD"]).0.trim().to_owned();
    // pushed from elsewhere, by path
    let cwd = scene.root.clone();
    let command = format!("git -C {} push origin feat/x", work.display());
    scene.bash_done(BETA_PID, SESSION_B, &cwd, &command, "", &updated);
    // a later commit does not move what was recorded; then an unchanged
    // repeat push by another session says "Everything up-to-date"
    git(&work, &["commit", "-q", "--allow-empty", "-m", "three"]);
    git(&work, &["reset", "-q", "--hard", "HEAD~1"]);
    let (_, nothing) = git(&work, &["push", "origin", "feat/x"]);
    assert!(nothing.contains("Everything up-to-date"), "{nothing}");
    scene.register(GAMMA_PID, SESSION_C, "gamma-name");
    scene.bash_done(
        GAMMA_PID,
        SESSION_C,
        &work,
        "git push origin feat/x",
        "",
        &nothing,
    );
    let lines = scene.pushes();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines.iter().all(|line| line.contains(SESSION_B)),
        "{lines:?}"
    );
    assert!(lines[0].ends_with(&first), "{lines:?}");
    assert!(lines[1].ends_with(&second), "{lines:?}");
    let (code, out, _) = scene.run(&["who-pushed", "feat/x"]);
    assert_eq!(code, 0);
    assert!(out.lines().all(|line| line.contains("beta-name")), "{out}");
    assert!(!out.contains("gamma-name"), "{out}");
    let (code, json, _) = scene.run(&["who-pushed", &second[..10], "--json"]);
    assert_eq!(code, 0);
    let rows: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(rows[0]["session"], SESSION_B);
    let (code, _, err) = scene.run(&["who-pushed", "feat/none"]);
    assert_eq!(code, 1);
    assert!(err.starts_with("no pushes recorded for feat/none"), "{err}");
}

#[test]
fn a_detached_land_is_credited_once_to_its_launcher_not_to_the_waiter() {
    let scene = Scene::new("land");
    let work = real_repo(&scene);
    let receipt = scene.root.join("receipt-1.json");
    let printed = format!(
        "landing in the background; then: amont land --wait '{}'\n",
        receipt.display()
    );
    scene.bash_done(
        BETA_PID,
        SESSION_B,
        &work,
        "amont land --detach",
        &printed,
        "",
    );
    // B clears; C waits on the receipt, twice
    scene.register(
        BETA_PID,
        "eeeeeeee-0000-4000-8000-000000000005",
        "beta-name",
    );
    scene.register(GAMMA_PID, SESSION_C, "gamma-name");
    std::fs::write(
        &receipt,
        r#"{"started":1,"branch":"feat/x","pushed":"0123456789abcdef0123456789abcdef01234567","status":"landed"}"#,
    )
    .unwrap();
    let wait = format!("amont land --wait '{}'", receipt.display());
    scene.bash_done(GAMMA_PID, SESSION_C, &work, &wait, "landed", "");
    scene.bash_done(GAMMA_PID, SESSION_C, &work, &wait, "landed", "");
    let lines = scene.pushes();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains(SESSION_B), "{lines:?}");
    assert!(lines[0].ends_with("0123456789abcdef0123456789abcdef01234567"));
}
