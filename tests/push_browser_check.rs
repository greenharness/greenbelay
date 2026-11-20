//! `push-browser-check`, end to end through the real binary: a push of
//! interface changes is advised unless this session (or one of its side
//! transcripts) completed a `browser-check` bound to the pushed tree.
//!
//! Each test builds a repository with a bare remote under an isolated root,
//! writes transcripts in the shapes real sessions leave, and pushes with a
//! `PreToolUse` Bash payload that names the transcript.

use std::path::{Path, PathBuf};
use std::process::Command;

const AGENT: &str = "browser-check";
const GOOD_REPORT: &str =
    "browser: playwright\nurl: http://localhost:3000/settings\nnot checked: none";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[derive(Debug, PartialEq)]
enum Said {
    Silent,
    Advise(String),
    Deny(String),
}

struct World {
    root: PathBuf,
    ids: std::cell::Cell<u32>,
}

impl World {
    fn new(name: &str) -> World {
        let thread: String = format!("{:?}", std::thread::current().id())
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        let root = std::env::temp_dir().join(format!(
            "greenbelay-push-browser-check-{name}-{}-{thread}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("claude").join("agents")).unwrap();
        std::fs::write(root.join("global"), "").unwrap();
        std::fs::write(
            root.join("claude").join("agents").join("browser-check.md"),
            "---\nname: browser-check\n---\n",
        )
        .unwrap();
        let world = World {
            root,
            ids: std::cell::Cell::new(0),
        };
        // `push-preview` would advise on the same pushes: this suite is
        // about the browser check alone.
        world.set_global("greenbelay.push-preview.stance", "observe");
        world.set_global("greenbelay.implementation-review.stance", "observe");
        world
    }

    /// `main` pushed with one file, `feat/x` checked out with `files`
    /// committed on top; the repository counts as having a user interface.
    fn repo(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = self.root.join(name);
        let remote = self.root.join(format!("{name}.git"));
        Command::new("git")
            .args(["init", "-q", "--bare", "--template="])
            .arg(&remote)
            .output()
            .expect("git init --bare");
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main", "--template=", "."]);
        git(&dir, &["config", "user.email", "t@t"]);
        git(&dir, &["config", "user.name", "t"]);
        git(&dir, &["config", "greenbelay.push-preview.ui", "true"]);
        git(
            &dir,
            &["remote", "add", "origin", &remote.display().to_string()],
        );
        self.commit(&dir, &[("README.md", "readme\n")]);
        git(&dir, &["push", "-q", "origin", "main"]);
        git(&dir, &["fetch", "-q", "origin"]);
        git(&dir, &["checkout", "-q", "-b", "feat/x"]);
        self.commit(&dir, files);
        dir
    }

    fn ui_repo(&self, name: &str) -> PathBuf {
        self.repo(name, &[("src/App.tsx", "export const A = () => 1;\n")])
    }

    fn commit(&self, dir: &Path, files: &[(&str, &str)]) -> String {
        for (path, body) in files {
            let file = dir.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, body).unwrap();
        }
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "c"]);
        git(dir, &["rev-parse", "HEAD"])
    }

    fn bin(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_greenbelay"));
        command
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude"))
            .env_remove("XDG_STATE_HOME")
            .env("GIT_CONFIG_GLOBAL", self.root.join("global"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GREENBELAY_OFF");
        command
    }

    fn set_global(&self, key: &str, value: &str) {
        let out = Command::new("git")
            .args(["config", "--file"])
            .arg(self.root.join("global"))
            .args([key, value])
            .output()
            .expect("git config");
        assert!(out.status.success());
    }

    fn id(&self) -> String {
        let n = self.ids.get() + 1;
        self.ids.set(n);
        format!("toolu_{n:04}")
    }

    fn hook(&self, payload: serde_json::Value) -> Said {
        use std::io::Write;
        let mut child = self
            .bin()
            .arg("hook")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "a decision exits 0: {:?} {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).unwrap();
        if stdout.trim().is_empty() {
            return Said::Silent;
        }
        let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        let o = &v["hookSpecificOutput"];
        if let Some(reason) = o["permissionDecisionReason"].as_str() {
            assert_eq!(o["permissionDecision"].as_str(), Some("deny"), "{stdout}");
            return Said::Deny(reason.to_string());
        }
        if let Some(text) = o["additionalContext"].as_str() {
            return Said::Advise(text.to_string());
        }
        panic!("unexpected decision: {stdout}");
    }

    fn push(&self, dir: &Path, transcript: Option<&Path>) -> Said {
        self.push_cmd(dir, transcript, "git push -u origin feat/x")
    }

    fn push_cmd(&self, dir: &Path, transcript: Option<&Path>, command: &str) -> Said {
        let mut payload = serde_json::json!({
            "hook_event_name": "PreToolUse", "tool_name": "Bash",
            "cwd": dir, "session_id": "s1", "prompt_id": "p1",
            "tool_use_id": self.id(), "permission_mode": "default",
            "tool_input": {"command": command}
        });
        if let Some(t) = transcript {
            payload["transcript_path"] = serde_json::json!(t);
        }
        self.hook(payload)
    }

    fn write_tool(&self, dir: &Path, path: &Path) -> Said {
        self.hook(serde_json::json!({
            "hook_event_name": "PreToolUse", "tool_name": "Write",
            "cwd": dir, "session_id": "s1", "permission_mode": "default",
            "tool_input": {"file_path": path, "content": "{}"}
        }))
    }

    fn tree_sha(&self, dir: &Path, args: &[&str]) -> String {
        let out = self
            .bin()
            .arg("tree-sha")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn block(&self, dir: &Path) -> String {
        self.tree_sha(dir, &["--block"])
    }

    fn tree(&self, dir: &Path) -> String {
        self.tree_sha(dir, &[])
    }

    fn store(&self) -> PathBuf {
        self.root
            .join("claude")
            .join("greenbelay")
            .join("push-browser-check")
    }

    fn pass_file(&self, repo: &str, tree: &str) -> PathBuf {
        self.store()
            .join("by-tree")
            .join(repo)
            .join(format!("{tree}.json"))
    }

    fn journal(&self, rule: &str) -> Vec<String> {
        std::fs::read_to_string(self.root.join("claude/greenbelay/journal.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("F ") && l.contains(&format!(" {rule} ")))
            .map(str::to_string)
            .collect()
    }

    fn transcript(&self, name: &str, t: &Transcript) -> PathBuf {
        let file = self.root.join(format!("{name}.jsonl"));
        std::fs::write(&file, t.lines.join("\n") + "\n").unwrap();
        file
    }

    /// A side transcript of the session whose own transcript is `name`:
    /// `<root>/<name>/subagents/<file>.jsonl`.
    fn side_transcript(&self, name: &str, file: &str, t: &Transcript) -> PathBuf {
        let dir = self.root.join(name).join("subagents");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{file}.jsonl"));
        std::fs::write(&path, t.lines.join("\n") + "\n").unwrap();
        path
    }

    /// Approve a landed plan the way the person's approval records it.
    fn approve_plan(&self, repo: &Path, plan_rel: &str) -> String {
        let shown = self
            .bin()
            .arg("plan-sha")
            .arg(repo.join(plan_rel))
            .output()
            .unwrap();
        let sha = String::from_utf8_lossy(&shown.stdout).trim().to_string();
        assert_eq!(sha.len(), 64, "plan-sha printed {sha:?}");
        let approved = self.root.join("claude/greenbelay/plan-approved");
        std::fs::create_dir_all(&approved).unwrap();
        std::fs::write(approved.join(&sha), "s\tp\t1\n").unwrap();
        sha
    }

    /// `repo` with a landed plan (under `docs/plans/`) and the UI change on
    /// top, branched from `origin/main`.
    fn planned_ui_repo(&self, name: &str, plan: &str) -> (PathBuf, &'static str) {
        let plan_rel = "docs/plans/2026-11-20-x.md";
        let repo = self.ui_repo(name);
        git(&repo, &["reset", "-q", "--hard", "origin/main"]);
        self.commit(&repo, &[(plan_rel, plan)]);
        self.commit(&repo, &[("src/App.tsx", "export const A = () => 1;\n")]);
        (repo, plan_rel)
    }
}

#[derive(Default, Clone)]
struct Transcript {
    lines: Vec<String>,
}

impl Transcript {
    /// A foreground launch of `agent` with `prompt` that completed with
    /// `result`.
    fn run(&mut self, w: &World, agent: &str, prompt: &str, result: &str) -> &mut Self {
        let id = w.id();
        self.lines.push(
            serde_json::json!({
                "type": "assistant",
                "message": {"role": "assistant", "content": [{
                    "type": "tool_use", "id": id, "name": "Agent",
                    "input": {"description": "check", "subagent_type": agent, "prompt": prompt}
                }]}
            })
            .to_string(),
        );
        self.lines.push(
            serde_json::json!({
                "type": "user",
                "message": {"role": "user", "content": [{
                    "type": "tool_result", "tool_use_id": id,
                    "content": [{"type": "text", "text": result}]
                }]},
                "toolUseResult": {"status": "completed", "agentType": agent, "content": []}
            })
            .to_string(),
        );
        self
    }

    /// A completed `browser-check` of `block` that reports `result`.
    fn check(&mut self, w: &World, block: &str, result: &str) -> &mut Self {
        self.run(
            w,
            AGENT,
            &format!("{block}\nLook at the settings page.\n"),
            result,
        )
    }
}

fn advised(said: &Said) -> &str {
    match said {
        Said::Advise(t) => t,
        other => panic!("expected advice, got {other:?}"),
    }
}

#[test]
fn a_ui_push_with_no_check_is_advised_with_the_exact_tree_block_on_its_own_line() {
    let w = World::new("missing");
    let repo = w.ui_repo("app");
    let empty = w.transcript("empty", &Transcript::default());
    let said = w.push(&repo, Some(&empty));
    let text = advised(&said);
    assert!(
        text.contains("no browser check ran in this session"),
        "{text}"
    );
    let block = w.block(&repo);
    assert!(
        text.lines().any(|l| l == block),
        "{block} alone on a line: {text}"
    );
    assert!(text.contains("tree-sha --block -C"), "{text}");
    assert!(!text.contains('?'), "asks no question: {text}");
    assert!(
        !text.to_lowercase().contains("skill"),
        "names no skill: {text}"
    );
    assert!(
        w.journal("push-browser-check")
            .last()
            .is_some_and(|l| l.contains("check=missing")),
        "{:?}",
        w.journal("push-browser-check")
    );
}

#[test]
fn a_check_bound_to_the_pushed_tree_is_silent_and_remembered() {
    let w = World::new("bound");
    let repo = w.ui_repo("app");
    let mut t = Transcript::default();
    t.check(&w, &w.block(&repo), GOOD_REPORT);
    let transcript = w.transcript("t", &t);
    assert_eq!(w.push(&repo, Some(&transcript)), Said::Silent);
    let tree = w.tree(&repo);
    assert!(
        w.pass_file("app", &tree).is_file(),
        "the pass is remembered"
    );
    // The remembered pass stands without the transcript.
    let empty = w.transcript("empty", &Transcript::default());
    assert_eq!(w.push(&repo, Some(&empty)), Said::Silent);
}

#[test]
fn a_check_of_an_older_tree_then_an_edit_is_advised_naming_both_trees() {
    let w = World::new("older");
    let repo = w.ui_repo("app");
    let old = w.tree(&repo);
    let mut t = Transcript::default();
    t.check(&w, &w.block(&repo), GOOD_REPORT);
    let transcript = w.transcript("t", &t);
    w.commit(&repo, &[("src/App.tsx", "export const A = () => 2;\n")]);
    let new = w.tree(&repo);
    let said = w.push(&repo, Some(&transcript));
    let text = advised(&said);
    assert!(
        text.contains(&format!(
            "the last browser check read tree {}, not {}",
            &old[..12],
            &new[..12]
        )),
        "{text}"
    );
    assert!(!w.pass_file("app", &old).exists());
}

#[test]
fn a_report_that_reached_no_page_is_not_a_check() {
    let w = World::new("unreachable");
    let repo = w.ui_repo("app");
    for (name, report) in [
        ("only", "unreachable: http://localhost:3000/"),
        (
            "browser",
            "browser: playwright\nunreachable: http://localhost:3000/",
        ),
        ("no-url", "browser: playwright\nnot checked: everything"),
    ] {
        let mut t = Transcript::default();
        t.check(&w, &w.block(&repo), report);
        let transcript = w.transcript(name, &t);
        let said = w.push(&repo, Some(&transcript));
        let text = advised(&said);
        assert!(text.contains("reached no page"), "{name}: {text}");
    }
    assert!(!w.pass_file("app", &w.tree(&repo)).exists());
}

#[test]
fn a_push_that_changes_no_interface_is_silent() {
    let w = World::new("non-ui");
    let repo = w.repo("app", &[("docs/notes.md", "notes\n")]);
    let empty = w.transcript("empty", &Transcript::default());
    assert_eq!(w.push(&repo, Some(&empty)), Said::Silent);
}

#[test]
fn a_test_only_ui_change_is_silent() {
    let w = World::new("test-only");
    let repo = w.repo("app", &[("src/App.test.tsx", "test('a', () => {});\n")]);
    let empty = w.transcript("empty", &Transcript::default());
    assert_eq!(w.push(&repo, Some(&empty)), Said::Silent);
}

#[test]
fn a_rebase_onto_a_moved_base_needs_a_new_check() {
    let w = World::new("rebase");
    let repo = w.ui_repo("app");
    let mut t = Transcript::default();
    t.check(&w, &w.block(&repo), GOOD_REPORT);
    let transcript = w.transcript("t", &t);
    let before = w.tree(&repo);
    git(&repo, &["checkout", "-q", "main"]);
    w.commit(&repo, &[("README.md", "readme, moved\n")]);
    git(&repo, &["push", "-q", "origin", "main"]);
    git(&repo, &["fetch", "-q", "origin"]);
    git(&repo, &["checkout", "-q", "feat/x"]);
    git(&repo, &["rebase", "-q", "origin/main"]);
    assert_ne!(w.tree(&repo), before, "the base moved the tree");
    let said = w.push(&repo, Some(&transcript));
    assert!(
        advised(&said).contains("the last browser check read tree"),
        "{said:?}"
    );
}

#[test]
fn only_the_hook_writes_a_pass_file() {
    let w = World::new("guard");
    let repo = w.ui_repo("app");
    let tree = w.tree(&repo);
    let file = w.pass_file("app", &tree);
    match w.write_tool(&repo, &file) {
        Said::Deny(text) => {
            assert!(text.contains("push-browser-check"), "{text}");
            assert!(text.contains("`browser-check`"), "its own remedy: {text}");
            assert!(text.contains("tree-sha --block"), "{text}");
            assert!(!text.contains("F4b"), "not the review's remedy: {text}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(!file.exists());
    let said = w.push_cmd(&repo, None, &format!("echo '{{}}' > {}", file.display()));
    match said {
        Said::Deny(text) => assert!(text.contains("push-browser-check"), "{text}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn an_approved_plan_declaring_none_passes_and_is_journalled_with_its_reason() {
    let w = World::new("declared");
    let plan = "---\nstatus: active\n---\n# X\n\n## Browser check\n\nnone: the page needs a login nobody can script.\n";
    let (repo, plan_rel) = w.planned_ui_repo("app", plan);
    let empty = w.transcript("empty", &Transcript::default());

    // Not approved yet: the declaration is the plan's, not the person's.
    let said = w.push(&repo, Some(&empty));
    assert!(advised(&said).contains("no browser check ran"), "{said:?}");

    let sha = w.approve_plan(&repo, plan_rel);
    assert_eq!(w.push(&repo, Some(&empty)), Said::Silent);
    let line = w
        .journal("push-browser-check")
        .into_iter()
        .find(|l| l.contains(" declared "))
        .expect("journalled");
    assert!(
        line.contains(&format!(
            "{plan_rel} commit={} sha={} none: the page needs a login nobody can script.",
            &git(&repo, &["rev-parse", "HEAD"])[..7],
            &sha[..12]
        )),
        "{line}"
    );
}

#[test]
fn a_pointer_plan_or_a_plan_without_the_section_does_not_pass() {
    let w = World::new("pointer");
    let pointer = "---\nstatus: active\ncanonical: elsewhere/plan.md\n---\n# X\n\n## Browser check\n\nnone: later.\n";
    let (repo, plan_rel) = w.planned_ui_repo("app", pointer);
    w.approve_plan(&repo, plan_rel);
    let empty = w.transcript("empty", &Transcript::default());
    let said = w.push(&repo, Some(&empty));
    assert!(advised(&said).contains("no browser check ran"), "{said:?}");

    let w = World::new("no-section");
    let plain = "---\nstatus: active\n---\n# X\n\n## Preview\n\nevidence: nothing to see.\n";
    let (repo, plan_rel) = w.planned_ui_repo("app", plain);
    w.approve_plan(&repo, plan_rel);
    let empty = w.transcript("empty", &Transcript::default());
    let said = w.push(&repo, Some(&empty));
    assert!(advised(&said).contains("no browser check ran"), "{said:?}");
}

#[test]
fn a_check_completed_in_a_fork_side_transcript_counts() {
    let w = World::new("fork");
    let repo = w.ui_repo("app");
    let main = w.transcript("main", &Transcript::default());
    let mut forked = Transcript::default();
    forked.check(&w, &w.block(&repo), GOOD_REPORT);
    w.side_transcript("main", "fork-1", &forked);
    assert_eq!(w.push(&repo, Some(&main)), Said::Silent);
}

#[test]
fn a_check_in_another_sessions_transcript_does_not_count() {
    let w = World::new("other-session");
    let repo = w.ui_repo("app");
    let main = w.transcript("main", &Transcript::default());
    let mut theirs = Transcript::default();
    theirs.check(&w, &w.block(&repo), GOOD_REPORT);
    w.transcript("other", &theirs);
    w.side_transcript("other", "fork-1", &theirs);
    let said = w.push(&repo, Some(&main));
    assert!(advised(&said).contains("no browser check ran"), "{said:?}");
}

#[test]
fn the_stance_ladder_deny_holds_a_missing_check() {
    let w = World::new("deny");
    let repo = w.ui_repo("app");
    let empty = w.transcript("empty", &Transcript::default());
    w.set_global("greenbelay.push-browser-check.stance", "deny");
    match w.push(&repo, Some(&empty)) {
        Said::Deny(text) => assert!(text.contains(&w.tree(&repo)), "names the tree: {text}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    // Under deny an unreadable push shape is held, not waved through.
    assert!(matches!(
        w.push_cmd(&repo, Some(&empty), "git push --all origin"),
        Said::Deny(_)
    ));
}

#[test]
fn a_missing_browser_check_agent_is_a_gap_said_as_advice_at_advise_and_under_deny() {
    let w = World::new("gap");
    let repo = w.ui_repo("app");
    let empty = w.transcript("empty", &Transcript::default());
    std::fs::remove_dir_all(w.root.join("claude").join("agents")).unwrap();
    for stance in ["advise", "deny"] {
        w.set_global("greenbelay.push-browser-check.stance", stance);
        let said = w.push(&repo, Some(&empty));
        let text = advised(&said);
        assert!(
            text.contains("no `browser-check` agent is installed"),
            "{stance}: {text}"
        );
        assert!(!text.contains("grh"), "names no sibling: {text}");
        assert!(
            w.journal("push-browser-check")
                .last()
                .is_some_and(|l| l.contains("check=gap")),
            "{stance}: {:?}",
            w.journal("push-browser-check")
        );
    }
}

#[test]
fn a_push_of_another_worktree_names_that_worktrees_tree() {
    let w = World::new("worktree");
    let repo = w.ui_repo("app");
    let other = w.root.join("app-wt-y");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/y",
            &other.display().to_string(),
            "origin/main",
        ],
    );
    git(&other, &["config", "user.email", "t@t"]);
    git(&other, &["config", "user.name", "t"]);
    w.commit(&other, &[("src/Other.tsx", "export const B = () => 2;\n")]);
    assert_ne!(w.tree(&repo), w.tree(&other));
    let empty = w.transcript("empty", &Transcript::default());
    let said = w.push_cmd(
        &repo,
        Some(&empty),
        &format!("git -C {} push -u origin feat/y", other.display()),
    );
    let text = advised(&said);
    let block = w.block(&other);
    assert!(text.lines().any(|l| l == block), "{block}: {text}");
    assert!(!text.contains(&w.tree(&repo)), "{text}");
    assert!(
        text.contains(&format!(
            "tree-sha --block -C {} {}",
            // The hook names the worktree by its resolved path.
            std::fs::canonicalize(&other).unwrap().display(),
            git(&other, &["rev-parse", "HEAD"])
        )),
        "{text}"
    );
}

#[test]
fn push_previews_evidence_journal_detail_is_unchanged() {
    let w = World::new("preview-evidence");
    w.set_global("greenbelay.push-preview.stance", "advise");
    let plan = "---\nstatus: active\n---\n# X\n\n## Preview\n\nevidence: nothing visible is left to judge.\n";
    let (repo, plan_rel) = w.planned_ui_repo("app", plan);
    let sha = w.approve_plan(&repo, plan_rel);
    let empty = w.transcript("empty", &Transcript::default());
    w.set_global("greenbelay.push-browser-check.stance", "observe");
    assert_eq!(w.push(&repo, Some(&empty)), Said::Silent);
    let line = w
        .journal("push-preview")
        .into_iter()
        .find(|l| l.contains(" evidence "))
        .expect("push-preview journals its evidence");
    let expected = format!(
        "{plan_rel} commit={} sha={} evidence: nothing visible is left to judge.",
        &git(&repo, &["rev-parse", "HEAD"])[..7],
        &sha[..12]
    );
    assert!(line.ends_with(&expected), "{line}\n{expected}");
}
