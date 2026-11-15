//! `plan-review stats --time` over a fixture: a plan root with one git
//! repository and its landed plans, a plan-review ledger, and transcripts,
//! all under a scratch directory. HOME, the Claude config directory, the
//! global git config and the plan root all point inside it, so nothing reads
//! or writes the machine's own.

use greenpiste_guard::civil;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Stdio};

const T: i64 = 1_790_000_000;
const R1: &str = "abababababababababababababababababababababababababababababababab";

fn iso(t: i64) -> String {
    let day = t.div_euclid(86_400);
    let s = t.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}.000Z",
        civil::iso(day),
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

struct Fx {
    dir: PathBuf,
}

impl Fx {
    fn new(name: &str) -> Fx {
        let dir = std::env::temp_dir().join(format!(
            "greenpiste-plan-time-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir); // absent on a first run: nothing to clear
        for d in [
            "home",
            "claude/amont-agent/plan-review/bodies",
            "claude/projects/p",
            "root/app/docs/plans",
        ] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("gitconfig"), "").unwrap();
        let fx = Fx { dir };
        fx.git(&["init", "-q", "-b", "main"], T - 5000);
        fx
    }
    fn root(&self) -> PathBuf {
        self.dir.join("root")
    }
    fn app(&self) -> PathBuf {
        self.root().join("app")
    }
    fn wt(&self, name: &str) -> String {
        self.root().join(name).display().to_string()
    }
    fn env(&self, c: &mut Command) {
        c.env("HOME", self.dir.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.dir.join("claude"))
            .env("XDG_STATE_HOME", self.dir.join("home/state"))
            .env("XDG_CONFIG_HOME", self.dir.join("home/config"))
            .env("GIT_CONFIG_GLOBAL", self.dir.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("AMONT_AGENT_PLAN_ROOT", self.root())
            .env_remove("AMONT_AGENT_OFF")
            .current_dir(&self.dir);
    }
    fn git(&self, args: &[&str], at: i64) {
        let mut c = Command::new("git");
        self.env(&mut c);
        let date = format!("@{at} +0000");
        let ok = c
            .arg("-C")
            .arg(self.app())
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }
    /// Writes a plan file and commits it at `at`.
    fn commit_plan(&self, name: &str, text: &str, at: i64) {
        std::fs::write(self.app().join("docs/plans").join(name), text).unwrap();
        self.git(&["add", "-A"], at);
        self.git(&["commit", "-q", "--no-verify", "-m", name], at);
    }
    fn transcript(&self, session: &str, lines: &[Value]) {
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(
            self.dir
                .join("claude/projects/p")
                .join(format!("{session}.jsonl")),
            text,
        )
        .unwrap();
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_greenpiste"));
        self.env(&mut c);
        c.args(["plan-review", "stats"]);
        c
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        self.command().args(args).output().unwrap()
    }
}

fn call(t: i64, cwd: &str, id: &str, name: &str, cmd: &str) -> Value {
    json!({"type": "assistant", "timestamp": iso(t), "cwd": cwd,
           "message": {"content": [{"type": "tool_use", "id": id, "name": name, "input": {"command": cmd}}]}})
}
fn result(t: i64, cwd: &str, id: &str) -> Value {
    json!({"type": "user", "timestamp": iso(t), "cwd": cwd,
           "message": {"content": [{"type": "tool_result", "tool_use_id": id, "content": "ok"}]}})
}
fn person(t: i64, cwd: &str) -> Value {
    json!({"type": "user", "timestamp": iso(t), "cwd": cwd, "message": {"content": "go on"}})
}

fn plan(branch: &str, status: &str, extra: &str) -> String {
    format!(
        "---\nstatus: {status}\nbranch: {branch}\nrepos: [app]\n---\n# A plan\n\n## Goal\nMeasure the thing for the people who wait on it.\n\n## Phases\n- [ ] Phase 1 — a: `src/a.rs`\n- [ ] Phase 2 — b: `src/b.rs`\n{extra}"
    )
}

/// The shared fixture: plan x (reviewed, done), y (unreviewed, unfinished,
/// on a reused worktree path), z (unfinished, on a detached worktree that
/// gained a branch), and one plan without `branch:`.
fn fixture(name: &str) -> Fx {
    let fx = Fx::new(name);
    let app = fx.app().display().to_string();
    let (x, rel) = (fx.wt("app-wt-x"), fx.wt("app-wt-rel"));
    let comment = format!(
        "\n<!-- panel: repos=app adds= reviewers=backend body-sha={} -->\n",
        &R1[..12]
    );
    fx.commit_plan("2026-01-01-z.md", &plan("feat/z", "active", ""), T - 2000);
    fx.commit_plan(
        "2026-01-02-x.md",
        &plan("feat/x", "active", &comment),
        T + 150,
    );
    fx.commit_plan("2026-01-03-y.md", &plan("feat/y", "active", ""), T + 2400);
    fx.commit_plan(
        "2026-01-04-nobranch.md",
        "---\nstatus: active\n---\n# N\n",
        T + 2450,
    );
    let grown = format!("- note: grown in review, with more words here\n{comment}");
    fx.commit_plan("2026-01-02-x.md", &plan("feat/x", "done", &grown), T + 5000);
    // The ledger: x's round 1 at T, its body saved.
    let row = json!({"schema": "amont-agent-review-outcome-v1", "role": "backend", "round": 1,
                     "body_sha": R1, "first_round_sha": R1, "recorded_at": T, "agent_id": "a1"});
    let store = fx.dir.join("claude/amont-agent/plan-review");
    std::fs::write(store.join("outcomes.jsonl"), format!("{row}\n")).unwrap();
    std::fs::write(
        store.join(format!("bodies/{R1}.md")),
        plan("feat/x", "active", ""),
    )
    .unwrap();
    // s0: a detached worktree gains feat/z, then a gate there (z, 100 s).
    fx.transcript(
        "s0",
        &[
            call(
                T - 1000,
                &app,
                "c0",
                "Bash",
                "git worktree add --detach ../app-wt-rel origin/main",
            ),
            call(
                T - 900,
                &app,
                "c1",
                "Bash",
                "cd ../app-wt-rel && git switch -c feat/z",
            ),
            call(T - 800, &rel, "c2", "Bash", "cargo test"),
            result(T - 700, &rel, "c2"),
        ],
    );
    // s1: plans x from its start. Before the add, an entry whose cwd is z's
    // worktree still counts for x: the pre-add rule wins.
    fx.transcript(
        "s1",
        &[
            call(T + 10, &app, "d0", "ExitPlanMode", ""),
            call(T + 50, &rel, "d1", "Bash", "cargo build"),
            result(T + 80, &rel, "d1"),
            call(
                T + 100,
                &app,
                "d2",
                "Bash",
                "git worktree add -q -b feat/x ../app-wt-x origin/main",
            ),
            result(T + 101, &app, "d2"),
            call(T + 200, &x, "d3", "Bash", "cargo test --workspace"),
            result(T + 800, &x, "d3"),
        ],
    );
    // s2: an agent in x's worktree, overlapping s1's test; then the person.
    fx.transcript(
        "s2",
        &[
            call(T + 500, &x, "e0", "Agent", ""),
            result(T + 1100, &x, "e0"),
            person(T + 3000, &x),
        ],
    );
    // s3: x's worktree is removed (a gate there is unattributed), then the
    // path is reused for feat/y.
    fx.transcript(
        "s3",
        &[
            call(
                T + 2000,
                &app,
                "f0",
                "Bash",
                "git worktree remove ../app-wt-x",
            ),
            call(T + 2100, &x, "f1", "Bash", "cargo build"),
            result(T + 2200, &x, "f1"),
            call(
                T + 2500,
                &app,
                "f2",
                "Bash",
                "git worktree add -b feat/y ../app-wt-x origin/main",
            ),
            call(T + 2600, &x, "f3", "Bash", "cargo test"),
            result(T + 2700, &x, "f3"),
        ],
    );
    fx
}

fn by_branch<'a>(v: &'a Value, list: &str, branch: &str) -> &'a Value {
    v[list]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["branch"] == branch)
        .unwrap_or_else(|| panic!("{branch} in {list}: {v}"))
}

#[test]
fn time_to_done_attributes_and_unions() {
    let fx = fixture("main");
    let out = fx.run(&["--time", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "--json keeps stderr quiet: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], 1);
    let x = by_branch(&v, "plans", "feat/x");
    assert_eq!(x["reviewed"], true);
    assert_eq!(x["elapsed_h"], 1.39, "T → T+5000");
    // [50,80] + union([200,800],[500,1100]) = 30 + 900 s.
    assert_eq!(x["blocked_h"], 0.26, "{x}");
    // 30 + 600 + 600 s.
    assert_eq!(x["resource_h"], 0.34, "{x}");
    assert_eq!(x["person_h"], 0.53, "1100 → 3000: {x}");
    assert_eq!(x["stops"], 1);
    assert!(
        x["words"]["final"].as_u64() > x["words"]["round1"].as_u64(),
        "{x}"
    );
    assert_eq!(x["phases"]["round1"], 2);
    let sessions: Vec<&str> = x["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(sessions, ["s1", "s2"]);
    // y: the reused path; unreviewed, unfinished.
    let y = by_branch(&v, "unfinished", "feat/y");
    assert_eq!(y["reviewed"], false);
    assert_eq!(y["resource_h"], 0.03, "2600 → 2700: {y}");
    // z: detached, then `switch -c`.
    let z = by_branch(&v, "unfinished", "feat/z");
    assert_eq!(z["resource_h"], 0.03, "{z}");
    // The gate in the removed worktree: an attribution failure, so unmapped.
    assert_eq!(v["unattributed"]["resource_h"], 0.03, "{v}");
    assert_eq!(v["unattributed"]["unmapped_blocked_h"], 0.03, "{v}");
    assert!(
        v["summary"]["unmapped_share"].as_f64().unwrap() > 0.0,
        "{v}"
    );
    assert_eq!(v["exclusions"]["no_branch"], 1);
    assert_eq!(v["summary"]["reviewed"], 1);
    assert_eq!(v["summary"]["done_check"], "insufficient");
}

/// A worktree added through a shell variable (`R=…; git -C $R worktree
/// add …`) from a session sitting in another repository's worktree: the
/// variable is not expanded, so the origin must not name the repository.
/// The worktree's `<repo>-wt-` name does (live data, 2026-11-15: a
/// relais worktree counted as amont-agent's).
#[test]
fn a_worktree_added_through_a_variable_keeps_its_repository() {
    let fx = fixture("var");
    let elsewhere = fx.wt("other-wt-q");
    let v2 = fx.wt("app-wt-v2");
    let cmd = format!(
        "R={}; git -C $R worktree add -q {} -b feat/x origin/main",
        fx.app().display(),
        v2
    );
    let root = fx.root().display().to_string();
    fx.transcript(
        "s4",
        &[
            // A worktree of another repository, `other`, where the session sits.
            call(
                T + 3000,
                &root,
                "g9",
                "Bash",
                "git worktree add -q ./other-wt-q -b feat/q origin/main",
            ),
            call(T + 3100, &elsewhere, "g0", "Bash", &cmd),
            result(T + 3101, &elsewhere, "g0"),
            call(T + 3200, &v2, "g1", "Bash", "cargo test"),
            result(T + 3300, &v2, "g1"),
        ],
    );
    let out = fx.run(&["--time", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let x = by_branch(&v, "plans", "feat/x");
    // The fixture's 1230 s of resource time, plus this 100 s gate.
    assert_eq!(x["resource_h"], 0.37, "{x}");
}

/// A plan file reused for a later plan: the ledger's later row carries the
/// first plan's `first_round_sha` (live data, 2026-11-15). The later plan
/// starts no earlier than the first one landed, and owns no round-1 body.
#[test]
fn a_reused_plan_file_does_not_share_its_start() {
    let fx = fixture("reuse");
    let r2 = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
    let comment = format!(
        "\n<!-- panel: repos=app adds= reviewers=backend body-sha={} -->\n",
        &r2[..12]
    );
    fx.commit_plan(
        "2026-01-05-w.md",
        &plan("feat/w", "active", &comment),
        T + 6000,
    );
    fx.commit_plan(
        "2026-01-05-w.md",
        &plan("feat/w", "done", &comment),
        T + 7000,
    );
    let store = fx.dir.join("claude/amont-agent/plan-review/outcomes.jsonl");
    let row = json!({"schema": "amont-agent-review-outcome-v1", "role": "backend", "round": 3,
                     "body_sha": r2, "first_round_sha": R1, "recorded_at": T + 5900, "agent_id": "a2"});
    let mut rows = std::fs::read_to_string(&store).unwrap();
    rows.push_str(&format!("{row}\n"));
    std::fs::write(&store, rows).unwrap();
    let out = fx.run(&["--time", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let w = by_branch(&v, "plans", "feat/w");
    // Bounded by x's landing (T+150), not x's round 1 (T): 6850 s.
    assert_eq!(w["elapsed_h"], 1.9, "{w}");
    assert!(w["words"].is_null(), "the round-1 body is x's: {w}");
    let x = by_branch(&v, "plans", "feat/x");
    assert_eq!(x["elapsed_h"], 1.39, "the first plan keeps its start: {x}");
}

/// A worktree a fork or subagent created: its `git worktree add` is only in
/// `<session>/subagents/*.jsonl` (live data, 2026-11-15: 2,481 side
/// transcripts add worktrees). The main session's gate there still counts.
#[test]
fn a_worktree_added_by_a_subagent_is_mapped() {
    let fx = fixture("side");
    let app = fx.app().display().to_string();
    let v3 = fx.wt("app-wt-v3");
    let side = fx.dir.join("claude/projects/p/s5/subagents");
    std::fs::create_dir_all(&side).unwrap();
    let add = call(
        T + 3100,
        &app,
        "h0",
        "Bash",
        "git worktree add -q -b feat/x ../app-wt-v3 origin/main",
    );
    std::fs::write(side.join("agent-1.jsonl"), format!("{add}\n")).unwrap();
    fx.transcript(
        "s5",
        &[
            call(T + 3200, &v3, "h1", "Bash", "cargo test"),
            result(T + 3300, &v3, "h1"),
        ],
    );
    let out = fx.run(&["--time", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let x = by_branch(&v, "plans", "feat/x");
    assert_eq!(
        x["resource_h"], 0.37,
        "the fixture's 1230 s plus this 100 s gate: {x}"
    );
}

/// Work in a plan's worktree after its `status: done` commit (release,
/// observe) is not the plan's time: blocked never exceeds elapsed.
#[test]
fn time_after_done_is_not_the_plans() {
    let fx = fixture("afterdone");
    let app = fx.app().display().to_string();
    let x2 = fx.wt("app-wt-x2");
    fx.transcript(
        "s6",
        &[
            call(
                T + 5050,
                &app,
                "k0",
                "Bash",
                "git worktree add -q -b feat/x ../app-wt-x2 origin/main",
            ),
            call(T + 5100, &x2, "k1", "Bash", "cargo test"),
            result(T + 5200, &x2, "k1"),
        ],
    );
    let out = fx.run(&["--time", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let x = by_branch(&v, "plans", "feat/x");
    assert_eq!(x["resource_h"], 0.34, "the post-done gate is not x's: {x}");
    assert_eq!(
        v["unattributed"]["resource_h"], 0.06,
        "it is unattributed: {v}"
    );
}

/// A resumed or forked session repeats earlier entries in its own file:
/// each entry counts once, by its `uuid`.
#[test]
fn a_repeated_entry_counts_once() {
    let fx = fixture("dedupe");
    let x = fx.wt("app-wt-x");
    let mut stop = call(T + 300, &x, "m0", "ExitPlanMode", "");
    stop["uuid"] = json!("u-shared");
    fx.transcript("s7", &[stop.clone()]);
    fx.transcript("s8", &[stop]);
    let out = fx.run(&["--time", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let px = by_branch(&v, "plans", "feat/x");
    assert_eq!(
        px["stops"], 2,
        "the fixture's one stop plus this one, once: {px}"
    );
}

#[test]
fn time_text_fits_80_columns_and_since_filters() {
    let fx = fixture("text");
    let out = fx.run(&["--time"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("app/feat/x"), "{text}");
    assert!(text.contains("summary: 1 finished"), "{text}");
    for l in text.lines() {
        assert!(l.chars().count() <= 80, "over 80 columns: {l}");
    }
    // A window after x's completion keeps the unfinished plans only.
    let out = fx.run(&["--time", "--json", "--since", "2027-01-01"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["plans"].as_array().unwrap().len(), 0);
    assert_eq!(v["unfinished"].as_array().unwrap().len(), 2);
}

#[test]
fn time_usage_errors_exit_2() {
    let fx = Fx::new("usage");
    for args in [
        &["--time", "--nosuchflag"][..],
        &["--nosuchflag"],
        &["--time", "--model", "opus"],
        &["--time", "--since", "2026-1-1"],
        &["--since", "2026-01-01"],
    ] {
        let out = fx.run(args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn a_closed_pipe_exits_zero() {
    let fx = fixture("pipe");
    let mut child = fx
        .command()
        .arg("--time")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    // `plan-review` keeps SIGPIPE ignored, so the write fails as EPIPE and
    // `main` treats it as the end of the reader: exit 0, never a signal
    // (141) or a panic (101).
    let code = child.wait().unwrap().code();
    assert_eq!(code, Some(0));
}
