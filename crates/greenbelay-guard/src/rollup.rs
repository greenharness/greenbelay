//! The weekly roll-up of what the journal saw: `rollup.log`, beside the
//! journal.
//!
//! `journal::rotate` folds the generation it is about to delete into this file
//! first, so firing counts outlive the journal's ten generations. Like the
//! journal it only counts: a wrong read here miscounts and never blocks.
//!
//! Format, one line each, the header first:
//!
//! ```text
//! greenbelay-rollup-v1
//! S <ts>                                       oldest record the first fold saw
//! W <ts>                                       watermark: newest record folded
//! R <week> <rule> <stance> <outcome> <arm> <n> firings; week is the Monday
//! C <week> <tool> <n>                          a closed week's calls, frozen
//! ```
//!
//! Any other line kind (`L` rows of a later release) is
//! carried through a rewrite unchanged, so an older binary never drops them.
//! A header naming another version is skipped whole: nothing is counted from
//! it and nothing is overwritten.
//!
//! The file is rewritten whole under `rollup.log.lock` (temp file, fsync,
//! rename), never appended, so a crash leaves the old file or the new one.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::civil;
use crate::journal::{self, Arm, Bucket};

pub const FORMAT: &str = "greenbelay-rollup-v1";

/// Weeks of `R` rows kept, newest first.
pub const WEEKS_KEPT: i64 = 104;

const LOCK_STALE_SECS: u64 = 60;
const SECONDS_PER_DAY: u64 = 86_400;
const DAYS_PER_WEEK: i64 = 7;

/// The roll-up that sits beside the journal at `journal_path`.
pub fn path_beside(journal_path: &Path) -> PathBuf {
    journal_path.with_file_name("rollup.log")
}

fn lock_path(rollup: &Path) -> PathBuf {
    let mut name = rollup.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

/// One counted bucket: a week's firings of one rule at one stance and outcome.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    /// The Monday of the UTC week.
    pub week: civil::Day,
    pub rule: String,
    pub stance: String,
    pub outcome: String,
    /// The arm stays in the key so a reader drops shadow.
    pub arm: Arm,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Rollup {
    /// `S`: the oldest record the first fold saw.
    pub covered_since: Option<u64>,
    /// `W`: the newest record timestamp folded so far.
    pub watermark: u64,
    pub rows: BTreeMap<Key, u64>,
    /// `C` rows: calls of one tool in a closed week, frozen the first time the
    /// week is computed and never rescanned.
    pub calls: BTreeMap<(civil::Day, String), u64>,
    /// Lines of a kind this version does not know, kept in order.
    pub carried: Vec<String>,
    /// Records the last [`Rollup::fold_text`] left out for a timestamp past its
    /// ceiling. Not written to the file.
    pub dropped_future: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    Rollup(Rollup),
    /// The header named a version this build does not read.
    UnknownVersion(String),
}

/// Why a fold did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldError {
    /// `rollup.log.lock` is held by someone else.
    Busy,
    /// An I/O failure; [`FoldError::why`] renders it as one whitespace-free token.
    Io(IoFailure),
}

/// An I/O failure: what kind, and on which file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoFailure {
    pub kind: std::io::ErrorKind,
    pub path: PathBuf,
}

impl IoFailure {
    fn new(error: &std::io::Error, path: &Path) -> IoFailure {
        IoFailure {
            kind: error.kind(),
            path: path.to_path_buf(),
        }
    }

    /// The kind as one whitespace-free token, as the journal writes it.
    pub fn why(&self) -> String {
        self.kind.to_string().replace(char::is_whitespace, "-")
    }
}

impl FoldError {
    /// The reason as a journal field.
    pub fn why(&self) -> String {
        match self {
            FoldError::Busy => "lock-busy".to_string(),
            FoldError::Io(failure) => failure.why(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folded {
    Counted,
    /// The roll-up has another format version; nothing was written.
    SkippedUnknownVersion,
    /// Counted, but this many records were stamped more than a day ahead of the
    /// clock and left out, so a skewed clock cannot move the watermark.
    DroppedFuture(u64),
}

/// Read `path`. A missing or empty file is an empty roll-up; an unreadable one
/// is an `Err` carrying the [`IoFailure`] (kind and path).
pub fn read(path: &Path) -> Result<Loaded, IoFailure> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Loaded::Rollup(Rollup::default()));
        }
        Err(e) => return Err(IoFailure::new(&e, path)),
    };
    Ok(parse(&text))
}

pub fn parse(text: &str) -> Loaded {
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Loaded::Rollup(Rollup::default());
    };
    if header != FORMAT {
        return Loaded::UnknownVersion(header.to_string());
    }
    let mut rollup = Rollup::default();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        if !rollup.take_known(line) {
            rollup.carried.push(line.to_string());
        }
    }
    Loaded::Rollup(rollup)
}

impl Rollup {
    /// Absorb a line of a kind this version knows; `false` for anything else.
    fn take_known(&mut self, line: &str) -> bool {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["S", ts] => ts.parse().map(|ts| self.covered_since = Some(ts)).is_ok(),
            ["W", ts] => ts.parse().map(|ts| self.watermark = ts).is_ok(),
            ["R", week, rule, stance, outcome, arm, count] => {
                let (Some(week), Ok(count)) = (civil::day_of(week), count.parse::<u64>()) else {
                    return false;
                };
                let Some(arm) = Arm::from_name(arm) else {
                    return false;
                };
                let key = Key {
                    week,
                    rule: (*rule).to_string(),
                    stance: (*stance).to_string(),
                    outcome: (*outcome).to_string(),
                    arm,
                };
                *self.rows.entry(key).or_default() += count;
                true
            }
            ["C", week, tool, count] => {
                let (Some(week), Ok(count)) = (civil::day_of(week), count.parse::<u64>()) else {
                    return false;
                };
                *self.calls.entry((week, (*tool).to_string())).or_default() += count;
                true
            }
            _ => false,
        }
    }

    pub fn render(&self) -> String {
        let mut out = format!("{FORMAT}\n");
        if let Some(ts) = self.covered_since {
            out.push_str(&format!("S {ts}\n"));
        }
        out.push_str(&format!("W {}\n", self.watermark));
        for (key, count) in &self.rows {
            out.push_str(&format!(
                "R {} {} {} {} {} {count}\n",
                civil::iso(key.week),
                key.rule,
                key.stance,
                key.outcome,
                key.arm.name()
            ));
        }
        for ((week, tool), count) in &self.calls {
            out.push_str(&format!("C {} {tool} {count}\n", civil::iso(*week)));
        }
        for line in &self.carried {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Count the journal records in `text` that are newer than the watermark
    /// this roll-up held when the fold began. Returns whether any record was
    /// seen at all. A record stamped after `ceiling` is a skewed clock's: it is
    /// counted in [`Rollup::dropped_future`] and moves neither the watermark nor
    /// the prune cutoff.
    pub fn fold_text(&mut self, text: &str, ceiling: u64) -> bool {
        let floor = self.watermark;
        let mut newest = None::<u64>;
        let mut oldest = None::<u64>;
        self.dropped_future = 0;
        for row in text.lines().filter_map(journal::fixed_fields) {
            if row.ts > ceiling {
                self.dropped_future += 1;
                continue;
            }
            newest = newest.max(Some(row.ts));
            oldest = Some(oldest.map_or(row.ts, |o| o.min(row.ts)));
            if row.ts <= floor {
                continue;
            }
            if Bucket::of(&row.stance, &row.outcome).is_none() {
                continue;
            }
            let key = Key {
                week: civil::week_start(i64::try_from(row.ts / SECONDS_PER_DAY).unwrap_or(0)),
                rule: row.rule,
                stance: row.stance,
                outcome: row.outcome,
                arm: row.arm,
            };
            *self.rows.entry(key).or_default() += 1;
        }
        let (Some(newest), Some(oldest)) = (newest, oldest) else {
            return false;
        };
        self.watermark = self.watermark.max(newest);
        self.covered_since.get_or_insert(oldest);
        self.prune();
        true
    }

    /// Drop `R` rows older than [`WEEKS_KEPT`] weeks before the newest one.
    fn prune(&mut self) {
        let newest = self
            .rows
            .keys()
            .map(|k| k.week)
            .chain(self.calls.keys().map(|(week, _)| *week))
            .max();
        let Some(newest) = newest else {
            return;
        };
        let cutoff = newest - DAYS_PER_WEEK * (WEEKS_KEPT - 1);
        self.rows.retain(|key, _| key.week >= cutoff);
        self.calls.retain(|(week, _), _| *week >= cutoff);
    }
}

/// The calls a report divides by: tool name counts per week.
pub type Calls = BTreeMap<civil::Day, BTreeMap<String, u64>>;

/// The tool whose call count is a rule's denominator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Bash,
    Named(&'static str),
    /// The rule fires on several tools: the sum of every tool's calls.
    AnyTool,
    /// No honest tool (a SessionStart or Stop event, an assertion): calls and
    /// rate are unknown, never a division by an unrelated count.
    None,
}

impl Tool {
    /// The JSON `tool` field; `None` is null.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Tool::Bash => Some("Bash"),
            Tool::Named(name) => Some(name),
            Tool::AnyTool => Some("any"),
            Tool::None => None,
        }
    }

    fn calls_in(self, week: &BTreeMap<String, u64>) -> Option<u64> {
        match self {
            Tool::Bash => Some(week.get("Bash").copied().unwrap_or(0)),
            Tool::Named(name) => Some(week.get(name).copied().unwrap_or(0)),
            Tool::AnyTool => Some(week.values().sum()),
            Tool::None => None,
        }
    }
}

/// Every rule and assertion id with the tool its rate divides by. A rule of
/// replay `exact`, `confirm` or `live` is a Bash rule; the ones the backtest
/// calls `not-shell` are named one by one. A test fails for an id of the rule
/// table that has no entry here, so a new rule cannot ship unmapped.
const RULE_TOOLS: &[(&str, Tool)] = &[
    ("pipe-to-tail", Tool::Bash),
    ("bare-stash-pop", Tool::Bash),
    ("cd-carries-over", Tool::Bash),
    ("gh-pr-merge-auto", Tool::Bash),
    ("block-reinstate", Tool::Bash),
    ("publish-without-skill", Tool::Bash),
    ("forge-merge-by-hand", Tool::Bash),
    ("no-verify", Tool::Bash),
    ("git-add-broad", Tool::Bash),
    ("stale-base", Tool::Bash),
    ("push-preflight", Tool::Bash),
    ("push-preview", Tool::Bash),
    ("plan-review-panel", Tool::Named("ExitPlanMode")),
    ("clear-at-boundary", Tool::None),
    ("plan-phases-open", Tool::None),
    ("implementation-review", Tool::Bash),
    ("push-browser-check", Tool::Bash),
    ("ask-unopened-page", Tool::Named("AskUserQuestion")),
    ("foreground-poll", Tool::Bash),
    ("unbounded-background-push", Tool::Bash),
    ("gui-open-unbounded", Tool::Bash),
    ("sed-in-place", Tool::Bash),
    ("kubectl-gitops", Tool::Bash),
    ("tag-after-commit", Tool::Bash),
    ("chain-with-and", Tool::Bash),
    ("release-tag-push", Tool::Bash),
    ("worktree-remove-force", Tool::Bash),
    ("amend-pushed", Tool::Bash),
    ("branch-force-delete", Tool::Bash),
    ("worktree-isolation", Tool::Bash),
    ("stdin-hang", Tool::Bash),
    ("glob-in-flag-value", Tool::Bash),
    ("glob-no-match", Tool::Bash),
    ("equals-separator", Tool::Bash),
    ("unsplit-expansion", Tool::Bash),
    ("path-operand-missing", Tool::Bash),
    ("stat-bsd-format", Tool::Bash),
    ("pkill-option-after-pattern", Tool::Bash),
    ("whole-file-dump", Tool::Bash),
    ("persisted-output-dump", Tool::AnyTool),
    ("file-reread", Tool::AnyTool),
    ("read-unbounded-large", Tool::Named("Read")),
    ("code-read-not-lsp", Tool::Named("Read")),
    ("symbol-grep-not-lsp", Tool::Bash),
    ("screenshot-in-main-session", Tool::AnyTool),
    ("relais-run-forks", Tool::AnyTool),
    ("agent-preferred-model", Tool::Named("Agent")),
    ("lint-suppression-added", Tool::AnyTool),
    ("agent-guidance-written", Tool::AnyTool),
    ("agent-guidance-shape", Tool::AnyTool),
    ("work-overlap", Tool::AnyTool),
    ("greenspotter-written", Tool::AnyTool),
    ("test-output-trimmed", Tool::Bash),
    ("workspace-root-write", Tool::Bash),
    ("heavy-build-unqueued", Tool::Bash),
    ("land-detached", Tool::Bash),
    ("suppressions-override", Tool::Bash),
    ("plan-names-no-paths", Tool::Named("ExitPlanMode")),
    ("secret-read", Tool::Bash),
    ("push-published", Tool::None),
    ("home-fingerprint-drift", Tool::None),
    ("push-landed", Tool::None),
    ("bug-nudge", Tool::None),
    ("land-retry", Tool::None),
];

/// The tool of a rule or assertion id, `None` for an id with no entry.
pub fn rule_tool(id: &str) -> Option<Tool> {
    RULE_TOOLS
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, tool)| *tool)
}

/// Every id [`rule_tool`] knows.
pub fn mapped_ids() -> impl Iterator<Item = &'static str> {
    RULE_TOOLS.iter().map(|(id, _)| *id)
}

/// A held `rollup.log.lock`, released on drop.
struct Lock(PathBuf);

impl Lock {
    /// Try once. A lock older than a minute belongs to a crashed holder and is
    /// removed.
    fn try_take(path: &Path) -> Option<Lock> {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        if opts.open(path).is_err() {
            let stale = fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > LOCK_STALE_SECS);
            if !stale || fs::remove_file(path).is_err() || opts.open(path).is_err() {
                return None;
            }
        }
        Some(Lock(path.to_path_buf()))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0); // a lock file that is already gone is the goal
    }
}

/// Fold the journal generation at `generation` into the roll-up at `rollup`.
/// Tries the lock once and never waits. Counting each record once is the
/// watermark's job, so folding the same generation twice is a no-op.
pub fn fold_generation(generation: &Path, rollup: &Path) -> Result<Folded, FoldError> {
    let _lock = Lock::try_take(&lock_path(rollup)).ok_or(FoldError::Busy)?;
    let text = match fs::read_to_string(generation) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Folded::Counted),
        Err(e) => return Err(FoldError::Io(IoFailure::new(&e, generation))),
    };
    let mut current = match read(rollup).map_err(FoldError::Io)? {
        Loaded::Rollup(current) => current,
        Loaded::UnknownVersion(_) => return Ok(Folded::SkippedUnknownVersion),
    };
    let ceiling = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(u64::MAX, |elapsed| {
            elapsed.as_secs().saturating_add(SECONDS_PER_DAY)
        }); // a clock before the epoch drops nothing
    if current.fold_text(&text, ceiling) {
        write_atomic(rollup, &current.render())
            .map_err(|err| FoldError::Io(IoFailure::new(&err, rollup)))?;
    }
    Ok(match current.dropped_future {
        0 => Folded::Counted,
        rows => Folded::DroppedFuture(rows),
    })
}

/// Write `contents` to a temp file beside `target`, fsync it, rename it over.
/// Leftover temps older than the lock's staleness are removed first; the
/// caller holds the lock.
fn write_atomic(target: &Path, contents: &str) -> std::io::Result<()> {
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    remove_stale_temps(target, &name);
    let temp_sequence = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = target.with_file_name(format!("{name}.new.{}.{temp_sequence}", std::process::id()));
    let written = (|| {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&temp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, target)?;
        // The rename is durable only once its directory entry is.
        // Best effort: the file is in place either way, so a failed sync is
        // not a failed write.
        if let Some(parent) = target.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            let _synced = fs::File::open(parent).and_then(|dir| dir.sync_all());
        }
        Ok(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp); // the write error is what is reported
    }
    written
}

fn remove_stale_temps(target: &Path, name: &str) {
    let Some(dir) = target.parent() else { return };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let prefix = format!("{name}.new.");
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() > LOCK_STALE_SECS);
        if old && entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = fs::remove_file(entry.path()); // a temp already gone is the goal
        }
    }
}

/// Why the roll-up could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The header named a version this build does not read.
    UnknownVersion(String),
    /// An I/O failure; the journal gets its kind as one token.
    Unreadable(IoFailure),
}

/// What the roll-up and the journal hold together.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Counted {
    pub rollup: Rollup,
    pub problem: Option<Problem>,
    /// Journal records newer than the roll-up's watermark: the ones no fold has
    /// counted yet. Both arms; a reader drops shadow.
    pub journal: Vec<journal::Row>,
}

/// Read the journal files, then the roll-up. In that order: a fold that lands
/// between the two reads has its rows in the roll-up and, because the roll-up
/// is read last, its new watermark filters them out of the journal rows, so
/// nothing is counted twice. An unreadable or foreign roll-up counts nothing
/// and filters nothing.
pub fn load(journal_paths: &[PathBuf], rollup_path: &Path) -> Counted {
    let rows = journal::rows_in(journal_paths);
    let (rollup, problem) = match read(rollup_path) {
        Ok(Loaded::Rollup(rollup)) => (rollup, None),
        Ok(Loaded::UnknownVersion(version)) => {
            (Rollup::default(), Some(Problem::UnknownVersion(version)))
        }
        Err(failure) => (Rollup::default(), Some(Problem::Unreadable(failure))),
    };
    let watermark = rollup.watermark;
    Counted {
        journal: rows.into_iter().filter(|row| row.ts > watermark).collect(),
        rollup,
        problem,
    }
}

/// The Monday of the UTC week `ts` falls in.
pub fn week_of(ts: u64) -> civil::Day {
    civil::week_start(i64::try_from(ts / SECONDS_PER_DAY).unwrap_or(0))
}

/// The oldest moment the counts reach back to: the roll-up's `S` or the
/// oldest journal record, whichever is older.
pub fn covered_since(counted: &Counted) -> Option<u64> {
    let oldest_journal = counted.journal.iter().map(|row| row.ts).min();
    [counted.rollup.covered_since, oldest_journal]
        .into_iter()
        .flatten()
        .min()
}

/// Closed weeks in `shown` whose calls can be frozen now: no `C` row yet, not
/// before the week of `covered_since`, and within a week of closing, because
/// Claude Code prunes old transcripts.
pub fn freezable_weeks(
    rollup: &Rollup,
    shown: &[civil::Day],
    today: civil::Day,
    covered_since: Option<u64>,
) -> Vec<civil::Day> {
    let Some(first) = covered_since.map(week_of) else {
        return Vec::new();
    };
    shown
        .iter()
        .copied()
        .filter(|&week| {
            let closed = week + DAYS_PER_WEEK <= today;
            let fresh = today < week + 2 * DAYS_PER_WEEK;
            closed
                && fresh
                && week >= first
                && !rollup.calls.keys().any(|(frozen, _)| *frozen == week)
        })
        .collect()
}

/// Freeze `weeks` from `scanned` into the roll-up at `path`. Takes only
/// `rollup.log.lock`, once, and re-reads the file under it so a fold that ran
/// during the scan keeps its rows. A week that gained `C` rows meanwhile is
/// left as it is. Returns the weeks written.
pub fn freeze_calls(
    path: &Path,
    scanned: &Calls,
    weeks: &[civil::Day],
) -> Result<Vec<civil::Day>, FoldError> {
    let _lock = Lock::try_take(&lock_path(path)).ok_or(FoldError::Busy)?;
    let mut current = match read(path).map_err(FoldError::Io)? {
        Loaded::Rollup(current) => current,
        Loaded::UnknownVersion(_) => return Ok(Vec::new()),
    };
    let mut written = Vec::new();
    for &week in weeks {
        let Some(tools) = scanned.get(&week) else {
            continue;
        };
        if current.calls.keys().any(|(frozen, _)| *frozen == week) {
            continue;
        }
        for (tool, count) in tools {
            current.calls.insert((week, tool.clone()), *count);
        }
        written.push(week);
    }
    if !written.is_empty() {
        write_atomic(path, &current.render())
            .map_err(|err| FoldError::Io(IoFailure::new(&err, path)))?;
    }
    Ok(written)
}

/// The calls a report reads: the frozen `C` rows, with `live` filling in any
/// week that has none (the current week, never stored).
pub fn merged_calls(rollup: &Rollup, live: &Calls) -> Calls {
    let mut merged = Calls::new();
    for ((week, tool), count) in &rollup.calls {
        merged
            .entry(*week)
            .or_default()
            .insert(tool.clone(), *count);
    }
    for (week, tools) in live {
        merged.entry(*week).or_insert_with(|| tools.clone());
    }
    merged
}

/// A rule the report covers, and the label `backtest` gives its replay.
#[derive(Debug, Clone, Copy)]
pub struct Known<'a> {
    pub id: &'a str,
    pub replay: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuleReport {
    pub rule: String,
    pub denied: u64,
    pub advised: u64,
    pub watched: u64,
    pub unconfirmed: u64,
    pub tool: Tool,
    pub calls: Option<u64>,
    pub replay: String,
}

impl RuleReport {
    pub fn firings(&self) -> u64 {
        self.denied + self.advised + self.watched + self.unconfirmed
    }

    /// Firings per 1,000 calls; unknown when the calls are, and when there are
    /// none to divide by.
    pub fn rate(&self) -> Option<f64> {
        self.calls
            .filter(|calls| *calls > 0)
            .map(|calls| self.firings() as f64 * 1000.0 / calls as f64)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeekReport {
    pub week: civil::Day,
    pub partial: bool,
    pub rules: Vec<RuleReport>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub covered_since: Option<civil::Day>,
    pub weeks: Vec<WeekReport>,
}

#[derive(Default, Clone, Copy)]
struct Buckets {
    denied: u64,
    advised: u64,
    watched: u64,
    unconfirmed: u64,
}

impl Buckets {
    fn add(&mut self, stance: &str, outcome: &str, count: u64) {
        match Bucket::of(stance, outcome) {
            Some(Bucket::Unconfirmed) => self.unconfirmed += count,
            Some(Bucket::Denied) => self.denied += count,
            Some(Bucket::Advised) => self.advised += count,
            Some(Bucket::Watched) => self.watched += count,
            None => {}
        }
    }
}

/// Weeks to show, newest first: the current one, then `closed` closed weeks, or
/// with `None` every week back to the first one with any data.
pub fn weeks_shown(
    counted: &Counted,
    calls: &Calls,
    today: civil::Day,
    closed: Option<usize>,
) -> Vec<civil::Day> {
    let current = civil::week_start(today);
    let earliest = match closed {
        Some(count) => current - DAYS_PER_WEEK * i64::try_from(count).unwrap_or(0),
        None => counted
            .rollup
            .rows
            .keys()
            .map(|key| key.week)
            .chain(counted.journal.iter().map(|row| week_of(row.ts)))
            .chain(calls.keys().copied())
            .chain(covered_since(counted).map(week_of))
            .min()
            .unwrap_or(current)
            .min(current),
    };
    let mut weeks = Vec::new();
    let mut week = current;
    while week >= earliest {
        weeks.push(week);
        week -= DAYS_PER_WEEK;
    }
    weeks
}

/// The report for `weeks` (newest first): one row per week and known rule,
/// live arm only.
pub fn build(
    counted: &Counted,
    calls: &Calls,
    known: &[Known],
    today: civil::Day,
    weeks: &[civil::Day],
) -> Report {
    let mut firings: BTreeMap<(civil::Day, &str), Buckets> = BTreeMap::new();
    for (key, count) in &counted.rollup.rows {
        if key.arm == Arm::Live {
            firings
                .entry((key.week, key.rule.as_str()))
                .or_default()
                .add(&key.stance, &key.outcome, *count);
        }
    }
    for row in &counted.journal {
        if row.arm == Arm::Live && Bucket::of(&row.stance, &row.outcome).is_some() {
            firings
                .entry((week_of(row.ts), row.rule.as_str()))
                .or_default()
                .add(&row.stance, &row.outcome, 1);
        }
    }
    let since = covered_since(counted);
    let first_week = since.map(week_of);
    let current = civil::week_start(today);
    let weeks = weeks
        .iter()
        .map(|&week| {
            // Firings are counted from `S` on, so a week that starts before it
            // is divided by a whole week of calls only if its rate is left out.
            let starts_before_coverage = Some(week) == first_week
                && since.is_some_and(|ts| ts > u64::try_from(week).unwrap_or(0) * SECONDS_PER_DAY);
            let partial = week == current || starts_before_coverage;
            let rules = known
                .iter()
                .map(|rule| {
                    let tool = rule_tool(rule.id).unwrap_or(Tool::None);
                    let found = firings.get(&(week, rule.id)).copied().unwrap_or_default();
                    RuleReport {
                        rule: rule.id.to_string(),
                        denied: found.denied,
                        advised: found.advised,
                        watched: found.watched,
                        unconfirmed: found.unconfirmed,
                        tool,
                        calls: if starts_before_coverage {
                            None
                        } else {
                            calls.get(&week).and_then(|tools| tool.calls_in(tools))
                        },
                        replay: rule.replay.to_string(),
                    }
                })
                .collect();
            WeekReport {
                week,
                partial,
                rules,
            }
        })
        .collect();
    Report {
        covered_since: since.map(|ts| i64::try_from(ts / SECONDS_PER_DAY).unwrap_or(0)),
        weeks,
    }
}

impl Report {
    /// The `--json` document; its field names are public interface.
    pub fn to_json(&self) -> String {
        use serde_json::json;
        let weeks: Vec<_> = self
            .weeks
            .iter()
            .map(|week| {
                let rules: Vec<_> = week
                    .rules
                    .iter()
                    .map(|rule| {
                        json!({
                            "rule": rule.rule,
                            "firings": rule.firings(),
                            "denied": rule.denied,
                            "advised": rule.advised,
                            "watched": rule.watched,
                            "unconfirmed": rule.unconfirmed,
                            "tool": rule.tool.label(),
                            "calls": rule.calls,
                            "rate": rule.rate().map(|rate| (rate * 10.0).round() / 10.0),
                            "replay": rule.replay,
                        })
                    })
                    .collect();
                json!({
                    "week": civil::iso(week.week),
                    "partial": week.partial,
                    "rules": rules,
                })
            })
            .collect();
        json!({
            "format": FORMAT,
            "covered_since": self.covered_since.map(civil::iso),
            "weeks": weeks,
        })
        .to_string()
    }

    /// The text table: `week rule firings calls rate/1000`, a row fits in 80
    /// columns while `rule_width` is the rule table's `ui::id_column`.
    pub fn to_text(&self, rule_width: usize) -> String {
        let mut out = format!(
            "{:<22}{:<rule_width$}{:>8}{:>8}{:>10}\n",
            "week", "rule", "firings", "calls", "rate/1000"
        );
        for week in &self.weeks {
            let label = if week.partial {
                format!("{} (partial)", civil::iso(week.week))
            } else {
                civil::iso(week.week)
            };
            for rule in &week.rules {
                let calls = rule
                    .calls
                    .map_or("-".to_string(), |count| count.to_string());
                let rate = rule
                    .rate()
                    .map_or("-".to_string(), |rate| format!("{rate:.1}"));
                out.push_str(&format!(
                    "{label:<22}{:<rule_width$}{:>8}{calls:>8}{rate:>10}\n",
                    rule.rule,
                    rule.firings()
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("greenbelay-rollup-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir); // a missing scratch dir is the normal case
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// 2023-11-14 22:13:20 UTC, a Tuesday; its week starts Monday 2023-11-13.
    const TUESDAY: u64 = 1_700_000_000;
    const MONDAY_DAY: civil::Day = 19_674;

    fn generation_text(rows: &[(u64, &str, &str)]) -> String {
        let mut text = format!("{}\n", journal::FORMAT);
        for (ts, rule, arm) in rows {
            text.push_str(&format!(
                "F {ts} {rule} deny denied sess repo default {arm} x\n"
            ));
        }
        text
    }

    fn live_count(rollup: &Rollup, rule: &str) -> u64 {
        rollup
            .rows
            .iter()
            .filter(|(k, _)| k.rule == rule && k.arm == Arm::Live)
            .map(|(_, n)| n)
            .sum()
    }

    fn load(path: &Path) -> Rollup {
        match read(path).expect("readable") {
            Loaded::Rollup(r) => r,
            Loaded::UnknownVersion(v) => panic!("unknown version {v}"),
        }
    }

    #[test]
    fn a_fold_counts_each_record_once_across_rotations_and_a_refold() {
        let dir = scratch("once");
        let rollup = dir.join("rollup.log");
        let first = dir.join("first");
        let second = dir.join("second");
        fs::write(
            &first,
            generation_text(&[(TUESDAY, "r", "arm=live"), (TUESDAY + 1, "r", "arm=live")]),
        )
        .unwrap();
        fs::write(&second, generation_text(&[(TUESDAY + 5, "r", "arm=live")])).unwrap();

        fold_generation(&first, &rollup).expect("first");
        assert_eq!(live_count(&load(&rollup), "r"), 2);
        fold_generation(&second, &rollup).expect("second");
        assert_eq!(live_count(&load(&rollup), "r"), 3);
        // A crash between the roll-up rename and the journal rename folds the
        // same generations again.
        fold_generation(&first, &rollup).expect("refold first");
        fold_generation(&second, &rollup).expect("refold second");
        let after = load(&rollup);
        assert_eq!(live_count(&after, "r"), 3);
        assert_eq!(after.watermark, TUESDAY + 5);
        assert_eq!(after.covered_since, Some(TUESDAY));
        assert!(after.rows.keys().all(|k| k.week == MONDAY_DAY));
        assert!(!lock_path(&rollup).exists(), "the lock is released");
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn shadow_rows_do_not_reach_the_live_keys() {
        let dir = scratch("shadow");
        let rollup = dir.join("rollup.log");
        let gen = dir.join("gen");
        fs::write(
            &gen,
            generation_text(&[(TUESDAY, "r", "arm=live"), (TUESDAY + 1, "r", "arm=shadow")]),
        )
        .unwrap();
        fold_generation(&gen, &rollup).expect("fold");
        assert_eq!(live_count(&load(&rollup), "r"), 1);
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn unknown_line_kinds_survive_a_fold() {
        let dir = scratch("carry");
        let rollup = dir.join("rollup.log");
        let gen = dir.join("gen");
        fs::write(
            &rollup,
            format!("{FORMAT}\nW 5\nC 2023-11-13 Bash 41210\nL 9 lost\n"),
        )
        .unwrap();
        fs::write(&gen, generation_text(&[(TUESDAY, "r", "arm=live")])).unwrap();
        fold_generation(&gen, &rollup).expect("fold");
        let text = fs::read_to_string(&rollup).unwrap();
        assert!(text.contains("\nC 2023-11-13 Bash 41210\n"), "{text}");
        assert!(text.contains("\nL 9 lost\n"), "{text}");
        assert!(text.contains("R 2023-11-13 r deny denied live 1"), "{text}");
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn an_unknown_header_version_is_skipped_untouched() {
        let dir = scratch("version");
        let rollup = dir.join("rollup.log");
        let gen = dir.join("gen");
        let original = "greenbelay-rollup-v2\nR whatever\n";
        fs::write(&rollup, original).unwrap();
        fs::write(&gen, generation_text(&[(TUESDAY, "r", "arm=live")])).unwrap();
        assert_eq!(
            fold_generation(&gen, &rollup),
            Ok(Folded::SkippedUnknownVersion)
        );
        assert_eq!(fs::read_to_string(&rollup).unwrap(), original);
        assert_eq!(
            read(&rollup),
            Ok(Loaded::UnknownVersion("greenbelay-rollup-v2".to_string()))
        );
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn a_held_lock_is_busy() {
        let dir = scratch("lock");
        let rollup = dir.join("rollup.log");
        let gen = dir.join("gen");
        fs::write(&gen, generation_text(&[(TUESDAY, "r", "arm=live")])).unwrap();
        fs::write(lock_path(&rollup), "").unwrap();
        assert_eq!(fold_generation(&gen, &rollup), Err(FoldError::Busy));
        assert!(
            lock_path(&rollup).exists(),
            "a busy lock is not ours to remove"
        );
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn a_stale_lock_is_taken() {
        let dir = scratch("stale");
        let rollup = dir.join("rollup.log");
        let gen = dir.join("gen");
        fs::write(&gen, generation_text(&[(TUESDAY, "r", "arm=live")])).unwrap();
        let lock = fs::File::create(lock_path(&rollup)).unwrap();
        lock.set_modified(
            std::time::SystemTime::now() - std::time::Duration::from_secs(LOCK_STALE_SECS + 5),
        )
        .unwrap();
        assert_eq!(fold_generation(&gen, &rollup), Ok(Folded::Counted));
        assert_eq!(live_count(&load(&rollup), "r"), 1);
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn a_future_record_neither_counts_nor_moves_the_watermark() {
        let mut rollup = Rollup::default();
        let text = generation_text(&[(TUESDAY, "r", "arm=live"), (TUESDAY * 3, "r", "arm=live")]);
        assert!(rollup.fold_text(&text, TUESDAY + SECONDS_PER_DAY));
        assert_eq!(rollup.dropped_future, 1);
        assert_eq!(rollup.watermark, TUESDAY);
        assert_eq!(live_count(&rollup, "r"), 1);
    }

    #[test]
    fn an_unreadable_rollup_is_an_io_failure() {
        let dir = scratch("unreadable");
        let rollup = dir.join("rollup.log");
        fs::create_dir(&rollup).unwrap();
        let gen = dir.join("gen");
        fs::write(&gen, generation_text(&[(TUESDAY, "r", "arm=live")])).unwrap();
        assert!(matches!(
            fold_generation(&gen, &rollup),
            Err(FoldError::Io(_))
        ));
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn only_a_hundred_and_four_weeks_are_kept() {
        let mut rollup = Rollup::default();
        let week = 7 * SECONDS_PER_DAY;
        let start = TUESDAY - 110 * week;
        let mut text = format!("{}\n", journal::FORMAT);
        for i in 0..=110 {
            text.push_str(&format!(
                "F {} r deny denied s repo default x\n",
                start + i * week
            ));
        }
        rollup.fold_text(&text, u64::MAX);
        let weeks: std::collections::BTreeSet<_> = rollup.rows.keys().map(|k| k.week).collect();
        assert_eq!(weeks.len(), 104);
    }

    fn transcript_line(id: &str, tool: &str, day: &str, is_sidechain: &str) -> String {
        format!(
            "{{\"timestamp\":\"{day}T10:00:00.000Z\",\"sessionId\":\"s\",\"cwd\":\"/p\",\
             \"isSidechain\":{is_sidechain},\"message\":{{\"content\":[{{\"type\":\"tool_use\",\
             \"id\":\"{id}\",\"name\":\"{tool}\",\"input\":{{}}}}]}}}}\n"
        )
    }

    /// A tool call in the main conversation's transcript.
    fn call_line(id: &str, tool: &str, day: &str) -> String {
        transcript_line(id, tool, day, "false")
    }

    /// A tool call in a subagent's transcript.
    fn sidechain_call_line(id: &str, tool: &str, day: &str) -> String {
        transcript_line(id, tool, day, "true")
    }

    fn transcript_roots(dir: &Path) -> crate::transcript::Roots {
        crate::transcript::Roots {
            dirs: vec![dir.join("projects")],
            how: crate::transcript::Discovery::Explicit,
        }
    }

    #[test]
    fn a_subagent_transcripts_calls_are_counted() {
        let dir = scratch("subagent");
        let sub = dir.join("projects/p/s/subagents");
        fs::create_dir_all(&sub).unwrap();
        fs::write(
            dir.join("projects/p/s.jsonl"),
            call_line("t1", "Bash", "2023-11-14"),
        )
        .unwrap();
        fs::write(
            sub.join("agent-a.jsonl"),
            format!(
                "{}{}",
                sidechain_call_line("t2", "Bash", "2023-11-15"),
                sidechain_call_line("t3", "Read", "2023-11-15")
            ),
        )
        .unwrap();
        let calls = crate::transcript::calls_by_week(&transcript_roots(&dir), None)
            .unwrap()
            .weeks;
        let week = &calls[&MONDAY_DAY];
        assert_eq!(week["Bash"], 2, "the subagent's Bash call counts");
        assert_eq!(week["Read"], 1);
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn a_closed_weeks_calls_are_frozen_and_not_recomputed() {
        let dir = scratch("freeze");
        let rollup = dir.join("rollup.log");
        fs::create_dir_all(dir.join("projects/p")).unwrap();
        let transcript = dir.join("projects/p/s.jsonl");
        fs::write(&transcript, call_line("t1", "Bash", "2023-11-14")).unwrap();
        let today = MONDAY_DAY + 8;
        let roots = transcript_roots(&dir);

        let empty = Rollup::default();
        let shown = [MONDAY_DAY];
        let covered = Some(TUESDAY);
        assert_eq!(freezable_weeks(&empty, &shown, today, covered), shown);
        let scanned = crate::transcript::calls_by_week(&roots, None)
            .unwrap()
            .weeks;
        assert_eq!(
            freeze_calls(&rollup, &scanned, &shown),
            Ok(vec![MONDAY_DAY])
        );

        fs::write(
            &transcript,
            format!(
                "{}{}",
                call_line("t1", "Bash", "2023-11-14"),
                call_line("t2", "Bash", "2023-11-15")
            ),
        )
        .unwrap();
        let frozen = load(&rollup);
        assert!(
            freezable_weeks(&frozen, &shown, today, covered).is_empty(),
            "a week with C rows is never rescanned"
        );
        let rescanned = crate::transcript::calls_by_week(&roots, None)
            .unwrap()
            .weeks;
        assert_eq!(
            rescanned[&MONDAY_DAY]["Bash"], 2,
            "the transcript did change"
        );
        assert_eq!(freeze_calls(&rollup, &rescanned, &shown), Ok(Vec::new()));
        let merged = merged_calls(&load(&rollup), &rescanned);
        assert_eq!(merged[&MONDAY_DAY]["Bash"], 1, "the frozen count holds");
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn a_week_is_not_frozen_late_or_before_coverage() {
        let empty = Rollup::default();
        let shown = [MONDAY_DAY];
        // Computed two weeks after closing: the transcripts may be pruned.
        assert!(freezable_weeks(&empty, &shown, MONDAY_DAY + 14, Some(TUESDAY)).is_empty());
        // Before the week of the first fold's oldest record.
        let later = TUESDAY + 14 * SECONDS_PER_DAY;
        assert!(freezable_weeks(&empty, &shown, MONDAY_DAY + 8, Some(later)).is_empty());
        // The open week is never frozen.
        assert!(freezable_weeks(&empty, &shown, MONDAY_DAY + 3, Some(TUESDAY)).is_empty());
    }

    #[test]
    fn rule_tool_covers_the_ids_it_lists_once() {
        let mut ids: Vec<&str> = mapped_ids().collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "an id is mapped twice");
        assert_eq!(rule_tool("pipe-to-tail"), Some(Tool::Bash));
        assert_eq!(rule_tool("no-such-rule"), None);
    }

    fn report_for(rows: &[(&str, u64)], calls: Option<u64>) -> Report {
        let mut counted = Counted::default();
        for (rule, count) in rows {
            counted.rollup.rows.insert(
                Key {
                    week: MONDAY_DAY,
                    rule: (*rule).to_string(),
                    stance: "deny".to_string(),
                    outcome: "denied".to_string(),
                    arm: Arm::Live,
                },
                *count,
            );
        }
        let mut all = Calls::new();
        if let Some(count) = calls {
            all.entry(MONDAY_DAY)
                .or_default()
                .insert("Bash".to_string(), count);
        }
        let known = [
            Known {
                id: "pipe-to-tail",
                replay: "exact",
            },
            Known {
                id: "clear-at-boundary",
                replay: "none",
            },
        ];
        build(&counted, &all, &known, MONDAY_DAY + 8, &[MONDAY_DAY])
    }

    #[test]
    fn a_rate_is_unknown_when_calls_are_unknown_or_zero_and_json_has_no_nan() {
        let report = report_for(&[("pipe-to-tail", 3), ("clear-at-boundary", 2)], Some(1000));
        let rules = &report.weeks[0].rules;
        assert_eq!(rules[0].rate(), Some(3.0));
        assert_eq!(rules[1].calls, None, "a rule with no tool has no calls");
        assert_eq!(rules[1].rate(), None);
        let none = report_for(&[("pipe-to-tail", 3)], None);
        assert_eq!(none.weeks[0].rules[0].calls, None);
        let zero = report_for(&[("pipe-to-tail", 3)], Some(0));
        assert_eq!(zero.weeks[0].rules[0].rate(), None);
        let json = zero.to_json();
        assert!(
            json.contains("\"rate\":null") && json.contains("\"calls\":0"),
            "{json}"
        );
        assert!(!json.contains("NaN") && !json.contains("inf"), "{json}");
        assert!(
            json.contains("\"tool\":\"Bash\"") && json.contains("\"tool\":null"),
            "{json}"
        );
    }

    #[test]
    fn a_week_starting_before_coverage_prints_no_calls() {
        let mut counted = Counted::default();
        // Coverage starts on the Wednesday of the Monday's week.
        counted.rollup.covered_since =
            Some(MONDAY_DAY as u64 * SECONDS_PER_DAY + 2 * SECONDS_PER_DAY);
        let mut all = Calls::new();
        all.entry(MONDAY_DAY)
            .or_default()
            .insert("Bash".to_string(), 500);
        let known = [Known {
            id: "pipe-to-tail",
            replay: "exact",
        }];
        let report = build(&counted, &all, &known, MONDAY_DAY + 8, &[MONDAY_DAY]);
        assert!(report.weeks[0].partial);
        assert_eq!(report.weeks[0].rules[0].calls, None);
        assert!(report.to_json().contains("\"rate\":null"));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_transcript_is_counted_so_nothing_is_frozen() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("unreadable-transcript");
        fs::create_dir_all(dir.join("projects/p")).unwrap();
        let locked = dir.join("projects/p/locked.jsonl");
        fs::write(&locked, call_line("t1", "Bash", "2023-11-14")).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let scanned = crate::transcript::calls_by_week(&transcript_roots(&dir), None).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o600)).unwrap();
        // Running as root reads the file anyway: nothing to assert then.
        if scanned.weeks.is_empty() {
            assert_eq!(scanned.unreadable, 1);
        }
    }

    #[test]
    fn a_transcript_older_than_the_floor_is_not_read() {
        let dir = scratch("mtime-floor");
        fs::create_dir_all(dir.join("projects/p")).unwrap();
        let old = dir.join("projects/p/old.jsonl");
        fs::write(&old, call_line("t1", "Bash", "2023-11-14")).unwrap();
        let long_ago = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        let skipped =
            crate::transcript::calls_by_week(&transcript_roots(&dir), Some(MONDAY_DAY)).unwrap();
        assert!(skipped.weeks.is_empty(), "{:?}", skipped.weeks);
        let read = crate::transcript::calls_by_week(&transcript_roots(&dir), None).unwrap();
        assert_eq!(read.weeks[&MONDAY_DAY]["Bash"], 1);
    }

    #[test]
    fn a_text_row_fits_in_eighty_columns() {
        let width = mapped_ids().map(str::len).max().unwrap_or(0) + 2;
        let mut report = report_for(&[("pipe-to-tail", 3_116)], Some(41_210));
        report.weeks[0].partial = true;
        for line in report.to_text(width).lines() {
            assert!(line.chars().count() <= 80, "`{line}` is wider than 80");
        }
    }

    #[test]
    fn a_fold_during_a_freeze_keeps_both_sets_of_rows() {
        let dir = scratch("interleave");
        let rollup = dir.join("rollup.log");
        let gen = dir.join("gen");
        fs::write(&gen, generation_text(&[(TUESDAY, "r", "arm=live")])).unwrap();
        // The freeze reads the file only once it holds the lock, so a fold that
        // landed during the scan is in what it rewrites.
        fold_generation(&gen, &rollup).expect("fold");
        let mut scanned = Calls::new();
        scanned
            .entry(MONDAY_DAY)
            .or_default()
            .insert("Bash".to_string(), 7);
        freeze_calls(&rollup, &scanned, &[MONDAY_DAY]).expect("freeze");
        let after = load(&rollup);
        assert_eq!(live_count(&after, "r"), 1);
        assert_eq!(after.calls[&(MONDAY_DAY, "Bash".to_string())], 7);
        assert!(!lock_path(&rollup).exists(), "the lock is released");
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }
}
