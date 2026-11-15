//! `greenpiste plan-review stats --time`: each plan's time to done
//! (decisions ADR-0070 `work.plan-time-to-done`; the measurement contract is
//! decisions `docs/programmes/time-to-done/t01-what-time-to-done-measures.md`).
//!
//! It reads three sources, and only these (`harness.cross-tool-reads`): the
//! Claude Code transcripts, this tool's own plan-review ledger, and git.
//!
//! - **Plans** are the `docs/plans/*.md` files with a `branch:` in their front
//!   matter, in every repository under the plan root, keyed by (repository,
//!   branch).
//! - **Start** is the earliest round-1 ledger row of the plan's first-round
//!   body. That body is found from the `body-sha=` of the plan's machine
//!   comment, else from the sha of the body as first committed. A plan the
//!   ledger does not know is `unreviewed`: it starts at its first commit and
//!   stays out of every median.
//! - **Done** is the first commit that adds `status: done` to the file.
//! - **Attribution**: a transcript entry belongs to the plan whose branch its
//!   `cwd` is checked out on. The map comes from the `git worktree add` /
//!   `remove` / `switch -c` commands in the transcripts, keyed by time, and
//!   from `git worktree list` for the worktrees that still exist.
//!   `gitBranch` is not used: it stays at the session's first branch.
//!   Before the add, the session that runs it belongs to the plan from the
//!   plan's start.
//! - **Blocked wait** is the union, counted once, of foreground gate, test or
//!   build calls, foreground agents, relais calls, and gaps over 10 minutes
//!   ended by a background notification.

use greenpiste_guard::civil;
use greenpiste_guard::names::BIN;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::IsTerminal as _;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

/// Bumped when a field changes meaning; retro-weekly checks it.
pub const SCHEMA: u64 = 1;
/// A gap longer than this, ended by a notification or by the person, is a
/// wait (the t01 contract).
const GAP: f64 = 600.0;
/// A plan whose body grew past this ratio in review "grew" (ADR-0070).
const GROWTH_BAR: f64 = 1.25;
/// Under this many finished plans with a round-1 body, the done check is
/// reported as insufficient (map.md: at least four plans).
const DONE_CHECK_MIN: usize = 4;
/// Transcripts older than this before the window still feed plans that
/// started earlier and finished inside it.
const LOOKBACK: f64 = 30.0 * 86_400.0;
/// Text column width for the plan name.
const NAME_W: usize = 38;

#[derive(Debug, Default)]
struct Options {
    since: Option<f64>,
    json: bool,
}

/// Parses `--time`'s own flags. `args` are those after `stats`.
fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--time" => {}
            "--json" => o.json = true,
            "--since" => {
                let v = it.next().ok_or("--since needs a date, YYYY-MM-DD")?;
                o.since = Some(day_secs(v)?);
            }
            "--model" => return Err("--model does not apply to --time".into()),
            _ if a.starts_with("--since=") => o.since = Some(day_secs(&a["--since=".len()..])?),
            _ if a.starts_with("--model=") => return Err("--model does not apply to --time".into()),
            _ => return Err(format!("unexpected argument `{a}`")),
        }
    }
    Ok(o)
}

fn day_secs(v: &str) -> Result<f64, String> {
    match civil::day_of(v) {
        Some(d) if v.len() == 10 => Ok(d as f64 * 86_400.0),
        _ => Err(format!("--since takes a date, YYYY-MM-DD, not `{v}`")),
    }
}

/// Seconds since the epoch of an RFC 3339 timestamp (`2026-11-15T08:01:02.5Z`),
/// UTC only, which is what the transcripts write. `None` for anything else.
pub(crate) fn epoch(ts: &str) -> Option<f64> {
    // Byte offsets below are only char boundaries in ASCII; a transcript is
    // untrusted input.
    if !ts.is_ascii() {
        return None;
    }
    let day = civil::day_of(ts)?;
    let b = ts.as_bytes();
    if b.len() < 19 || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |s: &str| s.parse::<f64>().ok();
    let (h, m) = (n(&ts[11..13])?, n(&ts[14..16])?);
    let end = ts[17..]
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map_or(ts.len(), |i| i + 17);
    let s = n(&ts[17..end])?;
    Some(day as f64 * 86_400.0 + h * 3600.0 + m * 60.0 + s)
}

/// One landed plan.
#[derive(Debug, Clone)]
struct Plan {
    repo: String,
    branch: String,
    path: PathBuf,
    landed: f64,
    done: Option<f64>,
    start: f64,
    reviewed: bool,
    words: Option<(usize, usize)>,
    phases: Option<(usize, usize)>,
    /// The first-round sha its review chain leads to.
    group: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct Tally {
    blocked: Vec<(f64, f64)>,
    person: Vec<(f64, f64)>,
    stops: u64,
    questions: u64,
    prs: u64,
    releases: u64,
    /// session id → (first, last) attributed entry.
    sessions: BTreeMap<String, (f64, f64)>,
}

#[derive(Debug, Default)]
struct Exclusions {
    no_branch: u64,
    not_active_or_done: u64,
    no_round1_body: u64,
    shared_first_round: u64,
    no_cwd: u64,
    git_failed: u64,
    transcripts_unreadable: u64,
}

/// Runs `plan-review stats --time`; returns stdout and the exit code.
pub fn run(args: &[String]) -> (String, ExitCode) {
    let opts = match parse(args) {
        Ok(o) => o,
        Err(why) => {
            eprint!("{BIN}: {why}\n\n{}", crate::review_outcomes::USAGE);
            return (String::new(), ExitCode::from(2));
        }
    };
    let Some(root) = crate::plan_review::root() else {
        eprintln!(
            "{BIN}: no plan root: nothing names where the repositories are. Run: {}",
            crate::plan_review::ROOT_REMEDY
        );
        return (String::new(), ExitCode::from(1));
    };
    let ledger = match ledger_rows() {
        Ok(r) => r,
        Err((path, why)) => {
            eprintln!(
                "{BIN}: cannot read ledger: {}: {why}. Run: ls -l {}",
                path.display(),
                path.display()
            );
            return (String::new(), ExitCode::from(1));
        }
    };
    let Some(projects) = greenpiste_guard::settings::config_dir().map(|d| d.join("projects"))
    else {
        eprintln!(
            "{BIN}: no home directory: the transcripts cannot be located. Set HOME (or CLAUDE_CONFIG_DIR) and run again."
        );
        return (String::new(), ExitCode::from(1));
    };
    let mut ex = Exclusions::default();
    let repos = repo_dirs(&root);
    let index = Ledger::new(&ledger);
    let bodies = greenpiste_guard::journal::dir().map(|d| d.join("plan-review").join("bodies"));
    let mut plans = Vec::new();
    for dir in &repos {
        plans.extend(plans_in(dir, &index, bodies.as_deref(), &mut ex));
    }
    split_shared_starts(&mut plans, &mut ex);
    let transcripts = match transcript_files(&projects) {
        Ok(t) => t,
        Err(why) => {
            eprintln!(
                "{BIN}: cannot read transcripts: {}: {why}. Run: ls {}",
                projects.display(),
                projects.display()
            );
            return (String::new(), ExitCode::from(1));
        }
    };
    let side = side_files(&projects);
    // The clock is read here, once, so the measure is a function of its inputs.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    let report = measure(&opts, &repos, plans, &transcripts, &side, now, &mut ex);
    let out = if opts.json {
        report.json(&ex)
    } else {
        report.text(&ex)
    };
    (out, ExitCode::SUCCESS)
}

fn ledger_rows() -> Result<Vec<Value>, (PathBuf, String)> {
    let Some(dir) = greenpiste_guard::journal::dir() else {
        return Err((PathBuf::from("~"), "no home directory".into()));
    };
    let path = dir.join("plan-review").join("outcomes.jsonl");
    crate::review_outcomes::read_rows(&path).map_err(|e| (path, e.to_string()))
}

/// The repositories under the plan root: directories holding a `.git`
/// directory, so linked worktrees (a `.git` file) are left out.
fn repo_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(".git").is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// What the plan-review ledger says about bodies and rounds.
struct Ledger {
    /// (body sha, first-round sha) of every reviewer row.
    reviews: Vec<(String, String)>,
    /// first-round sha → earliest round-1 `recorded_at`, else earliest of any round.
    starts: HashMap<String, (Option<u64>, u64)>,
    /// (after, before) of every plan-apply merge row.
    merges: Vec<(String, String)>,
    /// plan id → its first-round sha, from the rows that carry a `plan_id`.
    by_id: HashMap<crate::plan_id::PlanId, String>,
}

impl Ledger {
    fn new(rows: &[Value]) -> Ledger {
        let s = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).map(str::to_string);
        let mut l = Ledger {
            reviews: Vec::new(),
            starts: HashMap::new(),
            merges: Vec::new(),
            by_id: HashMap::new(),
        };
        for r in rows {
            if let (Some(after), Some(before)) = (s(r, "body_sha_after"), s(r, "body_sha_before")) {
                l.merges.push((after, before));
                continue;
            }
            let Some(body) = s(r, "body_sha") else {
                continue;
            };
            let first = s(r, "first_round_sha").unwrap_or_else(|| body.clone());
            if let Some(id) =
                s(r, "plan_id").and_then(|p| crate::plan_id::PlanId::try_from(p.as_str()).ok())
            {
                l.by_id.entry(id).or_insert_with(|| first.clone());
            }
            let at = r.get("recorded_at").and_then(Value::as_u64).unwrap_or(0);
            let round1 = r.get("round").and_then(Value::as_u64) == Some(1);
            let e = l.starts.entry(first.clone()).or_insert((None, u64::MAX));
            if round1 && at > 0 {
                e.0 = Some(e.0.map_or(at, |x| x.min(at)));
            }
            if at > 0 {
                e.1 = e.1.min(at);
            }
            l.reviews.push((body, first));
        }
        l
    }

    /// The first-round sha a body sha (or a 12-hex prefix of one) belongs to,
    /// following plan-apply merges back to a reviewed body.
    fn first_round(&self, key: &str) -> Option<String> {
        let mut key = key.to_string();
        for _ in 0..16 {
            if key.len() < 12 {
                return None;
            }
            if let Some((_, f)) = self
                .reviews
                .iter()
                .find(|(b, f)| b.starts_with(&key) || f.starts_with(&key))
            {
                return Some(f.clone());
            }
            match self.merges.iter().find(|(a, _)| a.starts_with(&key)) {
                Some((_, before)) => key = before.clone(),
                None => return None,
            }
        }
        None
    }

    fn start_of(&self, first: &str) -> Option<f64> {
        self.starts
            .get(first)
            .and_then(|(r1, any)| r1.or((*any != u64::MAX).then_some(*any)))
            .map(|t| t as f64)
    }
}

/// The `body-sha=` of a plan's machine comment, in the plan or its
/// `.reviews.md` sidecar.
fn comment_sha(plan: &Path) -> Option<String> {
    let side = plan.with_extension("").with_extension("reviews.md");
    [plan.to_path_buf(), side].iter().find_map(|p| {
        let text = std::fs::read_to_string(p).ok()?;
        let i = text.rfind("body-sha=")?;
        let hex: String = text[i + 9..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect();
        (hex.len() >= 12).then_some(hex)
    })
}

/// A front-matter field (`key: value`), when the file opens with `---`.
fn front(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for l in lines {
        if l.trim() == "---" {
            return None;
        }
        if let Some(v) = l.strip_prefix(key).and_then(|r| r.strip_prefix(':')) {
            let v = v
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['"', '\'']);
            return (!v.is_empty()).then(|| v.to_string());
        }
    }
    None
}

fn word_count(text: &str) -> usize {
    crate::plan_text::canonical(text).split_whitespace().count()
}

/// Distinct `Phase N` labels under the `## Phases` heading.
fn phase_count(text: &str) -> usize {
    let mut inside = false;
    let mut seen = std::collections::BTreeSet::new();
    for l in text.lines() {
        if l.starts_with("## ") {
            inside = l.trim() == "## Phases";
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(i) = l.find("Phase ") {
            let label: String = l[i + 6..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if label.starts_with(|c: char| c.is_ascii_digit()) {
                seen.insert(label);
            }
        }
    }
    seen.len()
}

/// The plans of one repository, with their landing, completion and start.
fn plans_in(dir: &Path, ledger: &Ledger, bodies: Option<&Path>, ex: &mut Exclusions) -> Vec<Plan> {
    let repo = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let plans_dir = dir.join("docs").join("plans");
    let Ok(entries) = std::fs::read_dir(&plans_dir) else {
        return Vec::new();
    };
    let Some(history) = plan_history(dir) else {
        eprintln!(
            "{BIN}: git failed: {}: the plans' history cannot be read. Run: git -C {} log -1",
            dir.display(),
            dir.display()
        );
        ex.git_failed += 1;
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") || name.ends_with(".reviews.md") || name == "README.md" {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if front(&text, "canonical").is_some() {
            continue; // a pointer file: the plan lives elsewhere
        }
        let Some(branch) = front(&text, "branch") else {
            ex.no_branch += 1;
            continue;
        };
        if !matches!(front(&text, "status").as_deref(), Some("active" | "done")) {
            ex.not_active_or_done += 1;
            continue;
        }
        let rel = format!("docs/plans/{name}");
        let Some((landed, first_commit, done)) = history.get(&rel).cloned() else {
            continue; // untracked: not landed yet
        };
        let key = comment_sha(&path).or_else(|| {
            greenpiste_guard::git::stdout_in(dir, &["show", &format!("{first_commit}:{rel}")])
                .map(|t| crate::plan_text::body_sha(&t))
        });
        // A plan with an id is its rows; a plan without one is found by its
        // machine comment's body sha (the landed body's as a fallback).
        let first = crate::plan_id::read(&text)
            .and_then(|id| ledger.by_id.get(&id).cloned())
            .or_else(|| key.as_deref().and_then(|k| ledger.first_round(k)));
        let start = first.as_deref().and_then(|f| ledger.start_of(f));
        let r1 = first
            .as_deref()
            .zip(bodies)
            .and_then(|(f, b)| std::fs::read_to_string(b.join(format!("{f}.md"))).ok());
        if first.is_some() && r1.is_none() {
            ex.no_round1_body += 1;
        }
        let words = r1.as_deref().map(|b| (word_count(b), word_count(&text)));
        let phases = r1.as_deref().map(|b| (phase_count(b), phase_count(&text)));
        out.push(Plan {
            repo: repo.clone(),
            branch,
            path,
            landed,
            done,
            start: start.unwrap_or(landed).min(landed),
            reviewed: start.is_some(),
            words,
            phases,
            group: first,
        });
    }
    out
}

/// A plan file under `~/.claude/plans` is reused for later plans, and the
/// ledger carries the first plan's `first_round_sha` on every later row
/// (live data, 2026-11-15: two plans a week apart shared one start, and the
/// later one's pre-add window claimed three days of other plans' time).
/// Plans that share a first-round group are ordered by landing: each one
/// starts no earlier than the previous one landed, and only the first one
/// owns the group's round-1 body, so the others have no word growth.
fn split_shared_starts(plans: &mut [Plan], ex: &mut Exclusions) {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, p) in plans.iter().enumerate() {
        if let Some(g) = &p.group {
            groups.entry(g.clone()).or_default().push(i);
        }
    }
    for mut idx in groups.into_values() {
        idx.sort_by(|&a, &b| plans[a].landed.total_cmp(&plans[b].landed));
        for w in 1..idx.len() {
            let prev = plans[idx[w - 1]].landed;
            let p = &mut plans[idx[w]];
            p.start = p.start.max(prev).min(p.landed);
            if p.words.take().is_some() {
                ex.shared_first_round += 1;
            }
            p.phases = None;
        }
    }
}

/// Per plan file: (landed time, first commit sha, done time), from one
/// `git log` over `docs/plans`.
type History = HashMap<String, (f64, String, Option<f64>)>;

fn plan_history(dir: &Path) -> Option<History> {
    let log = greenpiste_guard::git::stdout_in(
        dir,
        &[
            "log",
            "--reverse",
            "--no-merges",
            "--format=@@C %H %ct",
            "-p",
            "--unified=0",
            "--no-color",
            "--no-ext-diff",
            "--",
            "docs/plans",
        ],
    )?;
    let mut h: History = HashMap::new();
    let (mut sha, mut ct, mut file, mut added) = (String::new(), 0.0, String::new(), false);
    for l in log.lines() {
        if let Some(rest) = l.strip_prefix("@@C ") {
            let mut p = rest.split(' ');
            sha = p.next().unwrap_or("").to_string();
            ct = p.next().and_then(|t| t.parse().ok()).unwrap_or(0.0);
        } else if let Some(rest) = l.strip_prefix("diff --git a/") {
            file = rest.split(" b/").next().unwrap_or("").to_string();
            added = false;
        } else if l == "--- /dev/null" {
            added = true;
        } else if let Some(p) = l.strip_prefix("+++ b/") {
            if added {
                h.entry(p.to_string()).or_insert((ct, sha.clone(), None));
            }
        } else if l.trim_end() == "+status: done" {
            let e = h.entry(file.clone()).or_insert((ct, sha.clone(), None));
            if e.2.is_none() {
                e.2 = Some(ct);
            }
        }
    }
    Some(h)
}

/// The main transcripts, `projects/<project>/<session>.jsonl`, with mtimes.
fn transcript_files(projects: &Path) -> std::io::Result<Vec<(PathBuf, f64)>> {
    let mut out = Vec::new();
    for p in std::fs::read_dir(projects)?.flatten() {
        let Ok(files) = std::fs::read_dir(p.path()) else {
            continue;
        };
        for f in files.flatten() {
            let path = f.path();
            if path.extension().is_some_and(|e| e == "jsonl") {
                let m = f
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0.0, |d| d.as_secs_f64());
                out.push((path, m));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The side transcripts, `projects/<project>/<session>/subagents/*.jsonl`:
/// forks, subagents and relais workers, with mtimes.
fn side_files(projects: &Path) -> Vec<(PathBuf, f64)> {
    let mut out = Vec::new();
    let Ok(ps) = std::fs::read_dir(projects) else {
        return out;
    };
    for p in ps.flatten() {
        let Ok(sessions) = std::fs::read_dir(p.path()) else {
            continue;
        };
        for s in sessions.flatten() {
            let Ok(files) = std::fs::read_dir(s.path().join("subagents")) else {
                continue;
            };
            for f in files.flatten() {
                let path = f.path();
                if path.extension().is_some_and(|e| e == "jsonl") {
                    let m = f
                        .metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0.0, |d| d.as_secs_f64());
                    out.push((path, m));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A shell command split into its simple commands, each as words, with the
/// directory each one runs in (`cd` and `-C` followed). Quotes are dropped;
/// nothing is expanded. Good enough for git's own arguments.
fn simple_commands(cmd: &str, cwd: &Path) -> Vec<(PathBuf, Vec<String>)> {
    let mut out = Vec::new();
    let mut here = cwd.to_path_buf();
    for seg in cmd.split(['\n', ';', '|', '&', '(', ')']) {
        let words: Vec<String> = seg
            .split_whitespace()
            .map(|w| w.trim_matches(['"', '\'']).to_string())
            .filter(|w| !w.is_empty())
            .collect();
        if words.is_empty() {
            continue;
        }
        if words[0] == "cd" {
            if let Some(d) = words.get(1) {
                here = join(&here, d);
            }
            continue;
        }
        out.push((here.clone(), words));
    }
    out
}

/// `base` joined with `rel`, `..` and `.` resolved lexically; `~` expanded.
fn join(base: &Path, rel: &str) -> PathBuf {
    let p = if let Some(r) = rel.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(r)
    } else {
        base.join(rel)
    };
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// For a `git …` simple command: the directory it runs in (after `-C`) and
/// its subcommand words.
fn git_args<'a>(dir: &Path, words: &'a [String]) -> Option<(PathBuf, &'a [String])> {
    let i = words.iter().position(|w| w == "git")?;
    let mut dir = dir.to_path_buf();
    let mut j = i + 1;
    while j < words.len() && words[j].starts_with('-') {
        if (words[j] == "-C" || words[j] == "-c") && j + 1 < words.len() {
            if words[j] == "-C" {
                dir = join(&dir, &words[j + 1]);
            }
            j += 2;
        } else {
            j += 1;
        }
    }
    (j < words.len()).then(|| (dir, &words[j..]))
}

#[derive(Debug, Clone, PartialEq)]
enum WtEvent {
    /// A worktree at `path`, of the repository run from `origin`, on `branch`
    /// (`None` when detached).
    Add {
        path: PathBuf,
        origin: PathBuf,
        branch: Option<String>,
    },
    Remove {
        path: PathBuf,
    },
    /// A branch created where the command ran (`switch -c` / `checkout -b`).
    Switch {
        dir: PathBuf,
        branch: String,
    },
}

/// The worktree events one command carries.
fn worktree_events(cmd: &str, cwd: &Path) -> Vec<WtEvent> {
    let mut out = Vec::new();
    for (dir, words) in simple_commands(cmd, cwd) {
        let Some((dir, sub)) = git_args(&dir, &words) else {
            continue;
        };
        match sub.first().map(String::as_str) {
            Some("worktree") if sub.get(1).is_some_and(|s| s == "add") => {
                let (mut branch, mut path, mut k) = (None, None, 2);
                while k < sub.len() {
                    match sub[k].as_str() {
                        "-b" | "-B" | "--orphan" => {
                            branch = sub.get(k + 1).cloned();
                            k += 2;
                        }
                        "--reason" => k += 2,
                        a if a.starts_with('-') => k += 1,
                        a => {
                            if path.is_none() {
                                path = Some(join(&dir, a));
                            } else if branch.is_none()
                                && !a.contains('/')
                                && !sub.iter().any(|w| w == "--detach")
                            {
                                // `worktree add <path> <existing-branch>`
                                branch = Some(a.to_string());
                            }
                            k += 1;
                        }
                    }
                }
                if let Some(path) = path {
                    out.push(WtEvent::Add {
                        path,
                        origin: dir.clone(),
                        branch,
                    });
                }
            }
            Some("worktree") if sub.get(1).is_some_and(|s| s == "remove") => {
                if let Some(p) = sub[2..].iter().find(|a| !a.starts_with('-')) {
                    out.push(WtEvent::Remove {
                        path: join(&dir, p),
                    });
                }
            }
            Some("switch") | Some("checkout") => {
                let flag = if sub[0] == "switch" {
                    ["-c", "--create"]
                } else {
                    ["-b", "-B"]
                };
                if let Some(k) = sub
                    .iter()
                    .position(|a| flag.contains(&a.as_str()) || a == "-C")
                {
                    if let Some(b) = sub.get(k + 1) {
                        out.push(WtEvent::Switch {
                            dir: dir.clone(),
                            branch: b.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Does this command run a gate, test or build (the t00 audit's set, with
/// greengate and greenpiste)?
fn is_gate(cmd: &str) -> bool {
    // A command that only reads or prints its words runs none of them.
    const READERS: [&str; 12] = [
        "grep", "rg", "ugrep", "echo", "printf", "cat", "sed", "awk", "jq", "head", "tail", "less",
    ];
    simple_commands(cmd, Path::new("/")).iter().any(|(_, w)| {
        if w.first().is_some_and(|c| READERS.contains(&c.as_str())) {
            return false;
        }
        w.iter().enumerate().any(|(i, a)| {
            let next = w.get(i + 1).map(String::as_str);
            match a.as_str() {
                "greengate" | "amont" => {
                    matches!(next, Some("land" | "rehearse" | "run" | "slot" | "gate"))
                }
                "cargo" => matches!(next, Some("test" | "build" | "clippy" | "nextest")),
                "go" => next == Some("test"),
                "yarn" | "pnpm" | "npm" | "npx" | "vitest" | "pytest" | "tsc" => true,
                "git" => git_args(Path::new("/"), &w[i..]).is_some_and(|(_, s)| {
                    matches!(s.first().map(String::as_str), Some("commit" | "push"))
                }),
                _ => false,
            }
        })
    })
}

fn is_pr(cmd: &str) -> bool {
    cmd.contains("gh pr create") || (cmd.contains("/pulls") && cmd.contains("POST"))
}

fn is_release(cmd: &str) -> bool {
    simple_commands(cmd, Path::new("/")).iter().any(|(_, w)| {
        git_args(Path::new("/"), w).is_some_and(|(_, s)| {
            s.first().is_some_and(|x| x == "tag")
                && s.iter()
                    .skip(1)
                    .any(|a| a.starts_with('v') && a[1..].starts_with(|c: char| c.is_ascii_digit()))
        }) || (w.iter().any(|a| a == "land") && w.iter().any(|a| a == "--tag"))
    })
}

/// Where each worktree path pointed, over time.
#[derive(Default)]
struct WorktreeMap {
    /// path → (time, repo, branch or None when removed / detached).
    events: HashMap<PathBuf, Vec<(f64, String, Option<String>)>>,
    /// The worktrees that exist now: path → (repo, branch).
    live: HashMap<PathBuf, (String, String)>,
}

impl WorktreeMap {
    fn add(&mut self, t: f64, path: PathBuf, repo: String, branch: Option<String>) {
        self.events.entry(path).or_default().push((t, repo, branch));
    }

    fn sort(&mut self) {
        for v in self.events.values_mut() {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
    }

    /// The worktree path that holds `dir` (the longest known prefix).
    fn holder<'a>(&self, dir: &'a Path) -> Option<&'a Path> {
        dir.ancestors()
            .find(|a| self.events.contains_key(*a) || self.live.contains_key(*a))
    }

    /// (repo, branch) checked out at `dir` at time `t`.
    fn at(&self, dir: &Path, t: f64) -> Option<(String, String)> {
        let holder = self.holder(dir)?;
        if let Some(ev) = self.events.get(holder) {
            // Before the first recorded event the path held something else:
            // today's branch says nothing about then.
            let (_, repo, branch) = ev.iter().rev().find(|e| e.0 <= t)?;
            return branch.clone().map(|b| (repo.clone(), b));
        }
        self.live.get(holder).cloned()
    }

    /// The repository an origin directory belongs to.
    fn repo_of(&self, dir: &Path, repos: &[PathBuf], t: f64) -> Option<String> {
        if let Some(r) = repos.iter().find(|r| dir.starts_with(r)) {
            return r.file_name().map(|n| n.to_string_lossy().into_owned());
        }
        let holder = self.holder(dir)?;
        self.events
            .get(holder)
            .and_then(|ev| ev.iter().rev().find(|e| e.0 <= t).map(|e| e.1.clone()))
            .or_else(|| self.live.get(holder).map(|l| l.0.clone()))
    }
}

/// `git worktree list --porcelain` of every repository: the worktrees that
/// exist now.
fn live_worktrees(repos: &[PathBuf], git_failed: &mut u64) -> HashMap<PathBuf, (String, String)> {
    let mut out = HashMap::new();
    for r in repos {
        let name = r
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(list) = greenpiste_guard::git::stdout_in(r, &["worktree", "list", "--porcelain"])
        else {
            eprintln!(
                "{BIN}: git failed: {}: its worktrees cannot be listed. Run: git -C {} worktree list",
                r.display(),
                r.display()
            );
            *git_failed += 1;
            continue;
        };
        let mut path = None;
        for l in list.lines() {
            if let Some(p) = l.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if let Some(b) = l.strip_prefix("branch refs/heads/") {
                if let Some(p) = path.take() {
                    if p != *r {
                        out.insert(p, (name.clone(), b.to_string()));
                    }
                }
            }
        }
    }
    out
}

/// One transcript entry, reduced to what the measure needs.
#[derive(Debug, Clone, Default)]
struct Entry {
    t: f64,
    /// The entry's own id: a resumed or forked session repeats earlier
    /// entries, which count once.
    uuid: Option<String>,
    cwd: Option<PathBuf>,
    kind: Kind,
}

#[derive(Debug, Clone, Default, PartialEq)]
enum Kind {
    /// An assistant turn's tool calls: (id, name, Bash command, background).
    Calls(Vec<(String, String, String, bool)>),
    /// Tool results, by the call id they answer.
    Results(Vec<String>),
    /// A background notification (a task notification or queue operation).
    Notice,
    /// The person wrote.
    Person,
    #[default]
    Other,
}

fn entries(path: &Path) -> std::io::Result<Vec<Entry>> {
    entries_matching(path, None)
}

/// [`entries`], keeping only the lines that contain `needle`: a cheap
/// substring test before any JSON is parsed.
fn entries_matching(path: &Path, needle: Option<&str>) -> std::io::Result<Vec<Entry>> {
    use std::io::BufRead as _;
    let f = std::fs::File::open(path)?;
    let mut out = Vec::new();
    for line in std::io::BufReader::new(f).lines() {
        // A read error repeats on every call: stop reading this file.
        let Ok(line) = line else { break };
        if !line.contains("\"timestamp\"") || needle.is_some_and(|n| !line.contains(n)) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(t) = v.get("timestamp").and_then(Value::as_str).and_then(epoch) else {
            continue;
        };
        let cwd = v.get("cwd").and_then(Value::as_str).map(PathBuf::from);
        let uuid = v.get("uuid").and_then(Value::as_str).map(str::to_string);
        let content = v.pointer("/message/content");
        let kind = match v.get("type").and_then(Value::as_str) {
            Some("queue-operation") => Kind::Notice,
            Some("assistant") => {
                let calls: Vec<_> = content
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|c| c.get("type").and_then(Value::as_str) == Some("tool_use"))
                    .map(|c| {
                        let s =
                            |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                        let input = c.get("input");
                        let cmd = input
                            .and_then(|i| i.get("command"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let bg = input
                            .and_then(|i| i.get("run_in_background"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        (s("id"), s("name"), cmd, bg)
                    })
                    .collect();
                if calls.is_empty() {
                    Kind::Other
                } else {
                    Kind::Calls(calls)
                }
            }
            Some("user") => match content {
                Some(Value::Array(items)) => {
                    let ids: Vec<String> = items
                        .iter()
                        .filter(|c| c.get("type").and_then(Value::as_str) == Some("tool_result"))
                        .filter_map(|c| {
                            c.get("tool_use_id")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .collect();
                    if !ids.is_empty() {
                        Kind::Results(ids)
                    } else if items.iter().any(|c| {
                        c.get("text")
                            .and_then(Value::as_str)
                            .is_some_and(|t| t.contains("<task-notification>"))
                    }) {
                        Kind::Notice
                    } else if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
                        Kind::Other
                    } else {
                        Kind::Person
                    }
                }
                Some(Value::String(s)) if s.contains("<task-notification>") => Kind::Notice,
                Some(Value::String(_))
                    if v.get("isMeta").and_then(Value::as_bool) != Some(true) =>
                {
                    Kind::Person
                }
                _ => Kind::Other,
            },
            _ => Kind::Other,
        };
        out.push(Entry { t, uuid, cwd, kind });
    }
    Ok(out)
}

/// A session after its first read: its entries, and the worktree adds it ran.
struct Session {
    id: String,
    entries: Vec<Entry>,
    adds: Vec<(f64, String, String)>, // (time, repo, branch)
}

struct Report {
    finished: Vec<(Plan, Tally)>,
    unfinished: Vec<(Plan, Tally)>,
    unattributed: Tally,
    unmapped: Vec<(f64, f64)>,
    now: f64,
}

fn union(xs: &[(f64, f64)]) -> f64 {
    let mut v: Vec<(f64, f64)> = xs.iter().copied().filter(|(a, b)| b > a).collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut tot, mut cur): (f64, Option<(f64, f64)>) = (0.0, None);
    for (a, b) in v {
        match cur {
            Some((s, e)) if a <= e => cur = Some((s, e.max(b))),
            Some((s, e)) => {
                tot += e - s;
                cur = Some((a, b));
            }
            None => cur = Some((a, b)),
        }
    }
    tot + cur.map_or(0.0, |(s, e)| e - s)
}

fn sum(xs: &[(f64, f64)]) -> f64 {
    xs.iter().map(|(a, b)| (b - a).max(0.0)).sum()
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// The worktree events of one transcript's entries, folded into `map`; the
/// adds that name a branch are returned, for the pre-add windows.
fn map_events(
    es: &[Entry],
    repos: &[PathBuf],
    map: &mut WorktreeMap,
    pending_switch: &mut Vec<(f64, PathBuf, String)>,
) -> Vec<(f64, String, String)> {
    let mut adds = Vec::new();
    for e in es {
        let (Kind::Calls(calls), Some(cwd)) = (&e.kind, &e.cwd) else {
            continue;
        };
        for (_, name, cmd, _) in calls {
            if name != "Bash" || !cmd.contains("git") {
                continue;
            }
            for ev in worktree_events(cmd, cwd) {
                match ev {
                    WtEvent::Add {
                        path,
                        origin,
                        branch,
                    } => {
                        // The worktree's own `<repo>-wt-` name first: the
                        // origin can be a `git -C $VAR` nothing expands,
                        // which resolves inside another repository's
                        // worktree (live data, 2026-11-15).
                        let named = path.file_name().and_then(|b| {
                            b.to_string_lossy()
                                .split_once("-wt-")
                                .map(|(r, _)| r.to_string())
                        });
                        let known = named.clone().filter(|r| {
                            repos.iter().any(|p| {
                                p.file_name()
                                    .is_some_and(|n| n.to_string_lossy() == r.as_str())
                            })
                        });
                        let unexpanded = origin.to_string_lossy().contains('$');
                        let repo = known
                            .or_else(|| {
                                if unexpanded {
                                    None
                                } else {
                                    map.repo_of(&origin, repos, e.t)
                                }
                            })
                            .or(named);
                        let Some(repo) = repo else { continue };
                        if let Some(b) = &branch {
                            adds.push((e.t, repo.clone(), b.clone()));
                        }
                        map.add(e.t, path, repo, branch);
                    }
                    WtEvent::Remove { path } => {
                        let repo = map.repo_of(&path, repos, e.t).unwrap_or_default();
                        map.add(e.t, path, repo, None);
                    }
                    WtEvent::Switch { dir, branch } => pending_switch.push((e.t, dir, branch)),
                }
            }
        }
    }
    adds
}

fn measure(
    opts: &Options,
    repos: &[PathBuf],
    plans: Vec<Plan>,
    transcripts: &[(PathBuf, f64)],
    side: &[(PathBuf, f64)],
    now: f64,
    ex: &mut Exclusions,
) -> Report {
    // The plans in the window: done in it, or not done at all.
    let plans: Vec<Plan> = plans
        .into_iter()
        .filter(|p| match (p.done, opts.since) {
            (Some(d), Some(s)) => d >= s,
            _ => true,
        })
        .collect();
    let cutoff = opts.since.map(|s| {
        plans
            .iter()
            .filter(|p| p.done.is_some())
            .map(|p| p.start)
            .fold(s - LOOKBACK, f64::min)
    });
    let files: Vec<&(PathBuf, f64)> = transcripts
        .iter()
        .filter(|(_, m)| cutoff.is_none_or(|c| *m >= c))
        .collect();
    let progress = !opts.json && std::io::stderr().is_terminal();

    // Pass 1: read every session once; collect the worktree events.
    let mut map = WorktreeMap {
        live: live_worktrees(repos, &mut ex.git_failed),
        ..Default::default()
    };
    let mut sessions = Vec::new();
    let mut pending_switch: Vec<(f64, PathBuf, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (k, (path, _)) in files.iter().enumerate() {
        if progress {
            eprint!("\rtranscripts {}/{}", k + 1, files.len());
        }
        let Ok(mut es) = entries(path) else {
            ex.transcripts_unreadable += 1;
            continue;
        };
        es.retain(|e| e.uuid.as_ref().is_none_or(|u| seen.insert(u.clone())));
        let id = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let adds = map_events(&es, repos, &mut map, &mut pending_switch);
        sessions.push(Session {
            id,
            entries: es,
            adds,
        });
    }
    // Forks, subagents and relais workers create worktrees too: their
    // transcripts (`<session>/subagents/*.jsonl`) feed the map only. Their
    // waits are already the parent's foreground `Agent` call.
    for (path, _) in side.iter().filter(|(_, m)| cutoff.is_none_or(|c| *m >= c)) {
        let Ok(es) = entries_matching(path, Some("worktree")) else {
            continue;
        };
        map_events(&es, repos, &mut map, &mut pending_switch);
    }
    if progress {
        eprintln!();
    }
    map.sort();
    // A branch created inside a detached worktree names it from then on.
    for (t, dir, branch) in pending_switch {
        let Some(holder) = map.holder(&dir).map(Path::to_path_buf) else {
            continue;
        };
        let repo = map
            .events
            .get(&holder)
            .and_then(|ev| ev.iter().rev().find(|e| e.0 <= t))
            .map(|e| e.1.clone());
        let detached = map
            .events
            .get(&holder)
            .and_then(|ev| ev.iter().rev().find(|e| e.0 <= t))
            .is_some_and(|e| e.2.is_none());
        if let (Some(repo), true) = (repo, detached) {
            map.add(t, holder, repo, Some(branch));
        }
    }
    map.sort();

    // Plans by (repo, branch).
    let mut by_key: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, p) in plans.iter().enumerate() {
        by_key
            .entry((p.repo.clone(), p.branch.clone()))
            .or_default()
            .push(i);
    }
    let pick = |idx: &[usize], t: f64| -> Option<usize> {
        idx.iter()
            .copied()
            .filter(|&i| plans[i].start <= t)
            .max_by(|&a, &b| plans[a].start.total_cmp(&plans[b].start))
    };
    let mut tallies: Vec<Tally> = vec![Tally::default(); plans.len()];
    let mut unattributed = Tally::default();
    // The blocked intervals of `unattributed` that attribution failed on.
    let mut unmapped: Vec<(f64, f64)> = Vec::new();

    // Pass 2: attribute each session's entries and intervals.
    for s in &sessions {
        // The pre-add windows of this session: [plan start, add) → plan.
        let windows: Vec<(f64, f64, usize)> = s
            .adds
            .iter()
            .filter_map(|(t, repo, b)| {
                let i = pick(by_key.get(&(repo.clone(), b.clone()))?, *t)?;
                (plans[i].start <= *t).then_some((plans[i].start, *t, i))
            })
            .collect();
        let owner = |e: &Entry| -> Option<usize> {
            let pre = windows
                .iter()
                .filter(|(a, b, _)| *a <= e.t && e.t < *b)
                .max_by(|x, y| plans[x.2].start.total_cmp(&plans[y.2].start))
                .map(|w| w.2);
            pre.or_else(|| {
                let (repo, branch) = map.at(e.cwd.as_deref()?, e.t)?;
                pick(by_key.get(&(repo, branch))?, e.t)
            })
        };
        // An unattributed entry is a failure of attribution when it ran in
        // a worktree the map cannot name a branch for (its `worktree add` is
        // in no transcript left, or it was detached), or had no `cwd`; else
        // it ran on a branch with no landed plan, or in a primary checkout,
        // which no plan owns by design.
        let failed = |e: &Entry| -> bool {
            let Some(cwd) = e.cwd.as_deref() else {
                return true;
            };
            match map.holder(cwd) {
                Some(_) => map.at(cwd, e.t).is_none(),
                None => cwd
                    .components()
                    .any(|c| c.as_os_str().to_string_lossy().contains("-wt-")),
            }
        };
        let mut calls: HashMap<&str, (f64, Option<usize>, bool)> = HashMap::new();
        let mut prev: Option<(f64, Option<usize>, bool)> = None;
        for e in &s.entries {
            if e.cwd.is_none() && matches!(e.kind, Kind::Calls(_)) {
                ex.no_cwd += 1;
            }
            let who = owner(e);
            let lost = who.is_none() && failed(e);
            let gap_end = prev.filter(|(pt, _, _)| e.t - pt > GAP);
            match &e.kind {
                Kind::Calls(cs) => {
                    // Stops, questions, PRs and releases count for a plan only
                    // inside its span, as blocked and person time do.
                    let in_span = who
                        .filter(|&i| e.t >= plans[i].start && e.t <= plans[i].done.unwrap_or(now));
                    for (id, name, cmd, bg) in cs {
                        let tally = target(&mut tallies, &mut unattributed, in_span);
                        match name.as_str() {
                            "AskUserQuestion" => tally.questions += 1,
                            "ExitPlanMode" => tally.stops += 1,
                            "Bash" => {
                                if is_pr(cmd) {
                                    tally.prs += 1;
                                }
                                if is_release(cmd) {
                                    tally.releases += 1;
                                }
                            }
                            _ => {}
                        }
                        let waits = !bg
                            && ((name == "Bash" && is_gate(cmd))
                                || name == "Agent"
                                || name == "Task"
                                || name.starts_with("mcp__relais__"));
                        if waits {
                            calls.insert(id.as_str(), (e.t, who, lost));
                        }
                    }
                }
                Kind::Results(ids) => {
                    for id in ids {
                        if let Some((t0, w, l)) = calls.remove(id.as_str()) {
                            target(&mut tallies, &mut unattributed, w)
                                .blocked
                                .push((t0, e.t));
                            if l {
                                unmapped.push((t0, e.t));
                            }
                        }
                    }
                }
                Kind::Notice => {
                    if let Some((pt, w, l)) = gap_end {
                        target(&mut tallies, &mut unattributed, w)
                            .blocked
                            .push((pt, e.t));
                        if l {
                            unmapped.push((pt, e.t));
                        }
                    }
                }
                Kind::Person => {
                    if let Some((pt, w, _)) = gap_end {
                        target(&mut tallies, &mut unattributed, w)
                            .person
                            .push((pt, e.t));
                    }
                }
                Kind::Other => {}
            }
            if let Some(i) = who {
                let span = tallies[i]
                    .sessions
                    .entry(s.id.clone())
                    .or_insert((e.t, e.t));
                span.0 = span.0.min(e.t);
                span.1 = span.1.max(e.t);
            }
            prev = Some((e.t, who, lost));
        }
    }

    // A plan owns time inside its own span only: release and observe work in
    // its worktree after `done`, or an entry before its start, goes to the
    // unattributed row (and never makes blocked exceed elapsed).
    for (p, t) in plans.iter().zip(tallies.iter_mut()) {
        let end = p.done.unwrap_or(now);
        let (inside, outside) = clip(std::mem::take(&mut t.blocked), p.start, end);
        t.blocked = inside;
        unattributed.blocked.extend(outside);
        let (inside, outside) = clip(std::mem::take(&mut t.person), p.start, end);
        t.person = inside;
        unattributed.person.extend(outside);
    }
    let mut finished = Vec::new();
    let mut unfinished = Vec::new();
    for (p, t) in plans.into_iter().zip(tallies) {
        if p.done.is_some() {
            finished.push((p, t));
        } else {
            unfinished.push((p, t));
        }
    }
    finished.sort_by(|a, b| a.0.done.unwrap_or(0.0).total_cmp(&b.0.done.unwrap_or(0.0)));
    unfinished.sort_by(|a, b| b.0.landed.total_cmp(&a.0.landed));
    Report {
        finished,
        unfinished,
        unattributed,
        unmapped,
        now,
    }
}

/// Time intervals, `(from, to)` in epoch seconds.
type Spans = Vec<(f64, f64)>;

/// Each interval cut to `[a, b]`: the parts inside, and the parts outside.
fn clip(xs: Spans, a: f64, b: f64) -> (Spans, Spans) {
    // A span that ends before it starts (a rebased `done` commit) is empty:
    // nothing is inside, and nothing is counted twice outside.
    let b = b.max(a);
    let (mut inside, mut outside) = (Vec::new(), Vec::new());
    for (s, e) in xs {
        if s < a {
            outside.push((s, e.min(a)));
        }
        if e > b {
            outside.push((s.max(b), e));
        }
        let (cs, ce) = (s.max(a), e.min(b));
        if cs < ce {
            inside.push((cs, ce));
        }
    }
    (inside, outside)
}

fn target<'a>(tallies: &'a mut [Tally], un: &'a mut Tally, who: Option<usize>) -> &'a mut Tally {
    match who {
        Some(i) => &mut tallies[i],
        None => un,
    }
}

fn h(secs: f64) -> f64 {
    // `+ 0.0` turns a rounded -0.0 into 0.0, which JSON prints as `-0.0`.
    (secs / 3600.0 * 100.0).round() / 100.0 + 0.0
}

fn growth(p: &Plan) -> Option<f64> {
    p.words
        .and_then(|(a, b)| (a > 0).then(|| b as f64 / a as f64))
}

fn iso_time(t: f64) -> String {
    let day = (t / 86_400.0).floor() as i64;
    let s = t - day as f64 * 86_400.0;
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        civil::iso(day),
        (s / 3600.0) as u32,
        (s % 3600.0 / 60.0) as u32,
        (s % 60.0) as u32
    )
}

struct Summary {
    finished: usize,
    reviewed: usize,
    unreviewed: usize,
    round1_bodies: usize,
    median_elapsed_h: Option<f64>,
    median_blocked_h: Option<f64>,
    median_person_h: Option<f64>,
    median_growth: Option<f64>,
    grew_share: Option<f64>,
    unattributed_share: Option<f64>,
    unmapped_share: Option<f64>,
    insufficient: bool,
}

impl Report {
    fn summary(&self) -> Summary {
        let reviewed: Vec<&(Plan, Tally)> =
            self.finished.iter().filter(|(p, _)| p.reviewed).collect();
        let elapsed = |p: &Plan| p.done.unwrap_or(p.start) - p.start;
        let grows: Vec<f64> = reviewed.iter().filter_map(|(p, _)| growth(p)).collect();
        let all_blocked: f64 = self
            .finished
            .iter()
            .chain(&self.unfinished)
            .map(|(_, t)| union(&t.blocked))
            .sum::<f64>()
            + union(&self.unattributed.blocked);
        let round1_bodies = grows.len();
        Summary {
            finished: self.finished.len(),
            reviewed: reviewed.len(),
            unreviewed: self.finished.len() - reviewed.len(),
            round1_bodies,
            median_elapsed_h: median(reviewed.iter().map(|(p, _)| h(elapsed(p))).collect()),
            median_blocked_h: median(reviewed.iter().map(|(_, t)| h(union(&t.blocked))).collect()),
            median_person_h: median(reviewed.iter().map(|(_, t)| h(union(&t.person))).collect()),
            median_growth: median(grows.clone()),
            grew_share: (!grows.is_empty()).then(|| {
                grows.iter().filter(|g| **g > GROWTH_BAR).count() as f64 / grows.len() as f64
            }),
            unattributed_share: (all_blocked > 0.0)
                .then(|| union(&self.unattributed.blocked) / all_blocked),
            unmapped_share: (all_blocked > 0.0).then(|| union(&self.unmapped) / all_blocked),
            insufficient: round1_bodies < DONE_CHECK_MIN,
        }
    }

    fn plan_json(&self, p: &Plan, t: &Tally) -> Value {
        json!({
            "repo": p.repo,
            "branch": p.branch,
            "path": p.path.display().to_string(),
            "reviewed": p.reviewed,
            "start": iso_time(p.start),
            "landed": iso_time(p.landed),
            "done": p.done.map(iso_time),
            "elapsed_h": h(p.done.unwrap_or(self.now) - p.start),
            "blocked_h": h(union(&t.blocked)),
            "resource_h": h(sum(&t.blocked)),
            "person_h": h(union(&t.person)),
            "stops": t.stops,
            "questions": t.questions,
            "prs": t.prs,
            "releases": t.releases,
            "words": p.words.map(|(a, b)| json!({"round1": a, "final": b})),
            "phases": p.phases.map(|(a, b)| json!({"round1": a, "final": b})),
            "sessions": t.sessions.iter().map(|(id, (a, b))| json!({"id": id, "from": iso_time(*a), "to": iso_time(*b)})).collect::<Vec<_>>(),
        })
    }

    fn json(&self, ex: &Exclusions) -> String {
        let s = self.summary();
        let u = &self.unattributed;
        let v = json!({
            "schema": SCHEMA,
            "plans": self.finished.iter().map(|(p, t)| self.plan_json(p, t)).collect::<Vec<_>>(),
            "unfinished": self.unfinished.iter().map(|(p, t)| self.plan_json(p, t)).collect::<Vec<_>>(),
            "unattributed": {
                "blocked_h": h(union(&u.blocked)),
                "resource_h": h(sum(&u.blocked)),
                "person_h": h(union(&u.person)),
                "stops": u.stops,
                "questions": u.questions,
                "unmapped_blocked_h": h(union(&self.unmapped)),
            },
            "exclusions": exclusions_json(ex),
            "summary": {
                "finished": s.finished,
                "reviewed": s.reviewed,
                "unreviewed": s.unreviewed,
                "round1_bodies": s.round1_bodies,
                "median_elapsed_h": s.median_elapsed_h,
                "median_blocked_h": s.median_blocked_h,
                "median_person_h": s.median_person_h,
                "median_word_growth": s.median_growth.map(|g| (g * 100.0).round() / 100.0),
                "grew_past_bar_share": s.grew_share.map(|g| (g * 100.0).round() / 100.0),
                "unattributed_share": s.unattributed_share.map(|g| (g * 100.0).round() / 100.0),
                "unmapped_share": s.unmapped_share.map(|g| (g * 100.0).round() / 100.0),
                "done_check": if s.insufficient { "insufficient" } else { "computable" },
                "version": env!("CARGO_PKG_VERSION"),
            },
        });
        format!("{v}\n")
    }

    fn text(&self, ex: &Exclusions) -> String {
        let mut out = String::new();
        let name = |p: &Plan| {
            let full = format!("{}/{}", p.repo, p.branch);
            if full.chars().count() > NAME_W {
                format!("{}…", full.chars().take(NAME_W - 1).collect::<String>())
            } else {
                full
            }
        };
        let fmt_g = |p: &Plan| growth(p).map_or("-".to_string(), |g| format!("×{g:.2}"));
        let header = format!(
            "{:<NAME_W$} {:>8} {:>8} {:>7} {:>5} {:>6}\n",
            "plan", "elapsed", "blocked", "person", "stops", "words"
        );
        let row = |p: &Plan, t: &Tally, end: f64, mark: &str| {
            format!(
                "{:<NAME_W$} {:>7.1}h {:>7.1}h {:>6.1}h {:>5} {:>6}{mark}\n",
                name(p),
                h(end - p.start),
                h(union(&t.blocked)),
                h(union(&t.person)),
                t.stops,
                fmt_g(p),
            )
        };
        out.push_str(&format!("finished plans ({})\n", self.finished.len()));
        out.push_str(&header);
        for (p, t) in &self.finished {
            out.push_str(&row(
                p,
                t,
                p.done.unwrap_or(p.start),
                if p.reviewed { "" } else { " u" },
            ));
        }
        if self.finished.iter().any(|(p, _)| !p.reviewed) {
            out.push_str("  u = unreviewed: starts at landing, left out of the medians\n");
        }
        out.push_str(&format!(
            "\nunfinished plans ({}), elapsed so far\n",
            self.unfinished.len()
        ));
        for (p, t) in self.unfinished.iter().take(10) {
            out.push_str(&row(p, t, self.now, if p.reviewed { "" } else { " u" }));
        }
        if self.unfinished.len() > 10 {
            out.push_str(&format!(
                "  … {} more (--json lists them)\n",
                self.unfinished.len() - 10
            ));
        }
        let u = &self.unattributed;
        out.push_str(&format!(
            "\nunattributed: blocked {:.1}h, person {:.1}h, {} stops, {} questions\n",
            h(union(&u.blocked)),
            h(union(&u.person)),
            u.stops,
            u.questions
        ));
        let counts: Vec<String> = exclusions_json(ex)
            .as_object()
            .map(|o| {
                o.iter()
                    .filter(|(_, v)| v.as_u64() != Some(0))
                    .map(|(k, v)| {
                        let label = match k.as_str() {
                            "no_branch" => "no branch:",
                            "not_active_or_done" => "neither active nor done:",
                            "no_round1_body" => "no round-1 body:",
                            "shared_first_round" => "reused plan file:",
                            "calls_without_cwd" => "calls without cwd:",
                            "repos_git_failed" => "git failed:",
                            "transcripts_unreadable" => "unreadable transcripts:",
                            other => other,
                        };
                        format!("{label} {v}")
                    })
                    .collect()
            })
            .unwrap_or_default();
        if counts.is_empty() {
            out.push_str("exclusions: none\n");
        } else {
            out.push_str("exclusions:\n");
            let mut line = String::from(" ");
            let label_count = counts.len();
            for (k, c) in counts.into_iter().enumerate() {
                let c = if k + 1 < label_count {
                    format!("{c},")
                } else {
                    c
                };
                if line.len() + c.len() + 1 > 78 {
                    out.push_str(&line);
                    out.push('\n');
                    line = String::from(" ");
                }
                line.push(' ');
                line.push_str(&c);
            }
            out.push_str(&line);
            out.push('\n');
        }
        let s = self.summary();
        let f = |x: Option<f64>, unit: &str| x.map_or("-".to_string(), |v| format!("{v:.2}{unit}"));
        let pct = |x: Option<f64>| x.map_or("-".to_string(), |v| format!("{:.0}%", v * 100.0));
        out.push_str(&format!(
            "\nsummary: {} finished, {} reviewed, {} unreviewed, {} with a round-1 body{}\n  median elapsed {}, blocked {}, person {}\n  word growth {}, grew past ×{GROWTH_BAR}: {}\n  share of blocked: unattributed {}, unmapped (no worktree known) {}\n  greenpiste {}\n",
            s.finished,
            s.reviewed,
            s.unreviewed,
            s.round1_bodies,
            if s.insufficient { "\n  done check: insufficient, under 4 plans with a round-1 body" } else { "" },
            f(s.median_elapsed_h, "h"),
            f(s.median_blocked_h, "h"),
            f(s.median_person_h, "h"),
            s.median_growth.map_or("-".to_string(), |g| format!("×{g:.2}")),
            pct(s.grew_share),
            pct(s.unattributed_share),
            pct(s.unmapped_share),
            env!("CARGO_PKG_VERSION"),
        ));
        out
    }
}

fn exclusions_json(ex: &Exclusions) -> Value {
    json!({
        "no_branch": ex.no_branch,
        "not_active_or_done": ex.not_active_or_done,
        "no_round1_body": ex.no_round1_body,
        "shared_first_round": ex.shared_first_round,
        "calls_without_cwd": ex.no_cwd,
        "repos_git_failed": ex.git_failed,
        "transcripts_unreadable": ex.transcripts_unreadable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_reads_rfc3339_utc() {
        assert_eq!(epoch("1970-01-02T00:00:01Z"), Some(86_401.0));
        assert_eq!(epoch("1970-01-01T01:00:00.500Z"), Some(3600.5));
        assert_eq!(epoch("nope"), None);
        // A multibyte character at a sliced offset is no timestamp, never a panic.
        assert_eq!(epoch("2026-11-15T0é:00:00Z"), None);
        assert_eq!(epoch("2026-11-15T00:0é:00Z"), None);
    }

    #[test]
    fn union_counts_overlaps_once() {
        assert_eq!(union(&[(0.0, 10.0), (5.0, 15.0), (20.0, 30.0)]), 25.0);
        assert_eq!(sum(&[(0.0, 10.0), (5.0, 15.0)]), 20.0);
    }

    #[test]
    fn worktree_add_forms() {
        let cwd = Path::new("/r/app");
        let ev = worktree_events(
            "git worktree add -q ../app-wt-x -b feat/x origin/main && cd ../app-wt-x",
            cwd,
        );
        assert_eq!(
            ev,
            vec![WtEvent::Add {
                path: PathBuf::from("/r/app-wt-x"),
                origin: PathBuf::from("/r/app"),
                branch: Some("feat/x".into())
            }]
        );
        let ev = worktree_events(
            "git -C /r/app worktree add --detach ../app-wt-rel origin/main",
            Path::new("/tmp"),
        );
        assert!(
            matches!(&ev[0], WtEvent::Add { branch: None, path, .. } if path == Path::new("/r/app-wt-rel"))
        );
        let ev = worktree_events(
            "cd /r/app-wt-rel && git switch -c chore/release-1",
            Path::new("/"),
        );
        assert_eq!(
            ev,
            vec![WtEvent::Switch {
                dir: PathBuf::from("/r/app-wt-rel"),
                branch: "chore/release-1".into()
            }]
        );
        let ev = worktree_events("git worktree remove ../app-wt-x", cwd);
        assert_eq!(
            ev,
            vec![WtEvent::Remove {
                path: PathBuf::from("/r/app-wt-x")
            }]
        );
    }

    #[test]
    fn clip_cuts_each_interval_to_the_span() {
        // (interval, span start, span end, inside, outside)
        type Case = ((f64, f64), f64, f64, Spans, Spans);
        let cases: [Case; 5] = [
            ((0.0, 5.0), 10.0, 20.0, vec![], vec![(0.0, 5.0)]),
            ((12.0, 15.0), 10.0, 20.0, vec![(12.0, 15.0)], vec![]),
            ((25.0, 30.0), 10.0, 20.0, vec![], vec![(25.0, 30.0)]),
            (
                (5.0, 25.0),
                10.0,
                20.0,
                vec![(10.0, 20.0)],
                vec![(5.0, 10.0), (20.0, 25.0)],
            ),
            // `done` before `start`: an empty span; the interval counts once.
            (
                (10.0, 20.0),
                15.0,
                12.0,
                vec![],
                vec![(10.0, 15.0), (15.0, 20.0)],
            ),
        ];
        for (iv, a, b, inside, outside) in cases {
            let (i, o) = clip(vec![iv], a, b);
            assert_eq!(i, inside, "{iv:?} in [{a}, {b}]");
            assert_eq!(o, outside, "{iv:?} in [{a}, {b}]");
            let total: f64 = o.iter().chain(&i).map(|(s, e)| e - s).sum();
            assert_eq!(total, iv.1 - iv.0, "counted once: {iv:?}");
        }
    }

    #[test]
    fn gates_and_releases() {
        assert!(is_gate("cd x && git -C y commit -m z"));
        assert!(is_gate("greengate land --json"));
        assert!(is_gate("cargo test -p a"));
        assert!(!is_gate("git status"));
        assert!(
            !is_gate("grep cargo test"),
            "a word after grep is still a word"
        );
        assert!(is_release("git tag v1.2.0 abc && git push origin v1.2.0"));
        assert!(!is_release("git tag --points-at HEAD"));
    }

    #[test]
    fn front_matter_and_phases() {
        let t = "---\nstatus: done\nbranch: feat/x # c\n---\n# T\n## Phases\n- [x] Phase 1 — a\n- [ ] Phase 2a — b\n## Next\nPhase 9\n";
        assert_eq!(front(t, "branch").as_deref(), Some("feat/x"));
        assert_eq!(front(t, "status").as_deref(), Some("done"));
        assert_eq!(phase_count(t), 2);
    }

    #[test]
    fn parse_flags() {
        let a = |v: &[&str]| parse(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(a(&["--time", "--model", "x"]).is_err());
        assert!(a(&["--time", "--since", "2026-13-01"]).is_err());
        assert!(a(&["--time", "--since", "2026-11-01", "--json"]).is_ok());
        assert!(a(&["--time", "--nosuchflag"]).is_err());
    }
}
