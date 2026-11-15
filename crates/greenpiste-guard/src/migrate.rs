//! The one-time move of this machine's state from the old names to the new
//! ones (ADR-0068 `harness.renames`): a locked, resumable, reversible runner
//! over steps, and the state-directory step. Every path comes from [`Env`];
//! only [`Env::from_process`] reads the process's environment.

use crate::lock::lock_within;
use crate::names::{self, old};
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The two roots the migration works under.
pub struct Env {
    /// The Claude config directory (`~/.claude`); holds the markers and the
    /// lock, and the config-side data directory.
    pub config_dir: PathBuf,
    /// The new-name state directory; its parent holds the old-name sibling.
    pub state_root: PathBuf,
    /// The environment every git call runs under. `None` (what
    /// [`Env::from_process`] sets) means git inherits the process
    /// environment; `Some(list)` means each git command is built with the
    /// environment cleared, then exactly that list plus `PATH`.
    pub git_env: Option<Vec<(OsString, OsString)>>,
}

impl Env {
    /// The real machine's roots, `None` when no home can be found.
    pub fn from_process() -> Option<Env> {
        Some(Env {
            config_dir: crate::settings::config_dir()?,
            state_root: crate::settings::state_root()?.join(names::DATA_DIR),
            git_env: None,
        })
    }

    fn marker(&self, name: &str) -> PathBuf {
        self.config_dir.join(name)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Forward,
    Reverse,
}

/// How long a run waits for the lock: a hook never waits; an explicit command
/// waits up to a cap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wait {
    Zero,
    Capped(Duration),
}

/// One move. Both directions are idempotent and resumable.
pub trait Step {
    fn name(&self) -> &'static str;
    /// Do the step; the `Ok` text is a note ("" when there is none).
    fn forward(&self, env: &Env) -> Result<String, StepError>;
    fn reverse(&self, env: &Env) -> Result<String, StepError>;
    /// What the step would do in `direction`, one line each, reading only.
    fn plan(&self, env: &Env, direction: Direction) -> Result<Vec<String>, StepError>;
}

/// Why a step could not finish: the operation, the path when there is one,
/// and what went wrong underneath.
#[derive(Debug)]
pub struct StepError {
    operation: String,
    path: Option<PathBuf>,
    cause: Cause,
}

#[derive(Debug)]
enum Cause {
    Io(io::Error),
    Git(GitFailure),
    /// A condition the step refuses to work around; the operation text is the
    /// whole message.
    Refused,
    /// Several independent failures, reported one per line.
    Several(Vec<StepError>),
}

/// A failed git call, with what git said.
#[derive(Debug)]
pub struct GitFailure {
    message: String,
}

impl fmt::Display for GitFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GitFailure {}

impl StepError {
    /// An I/O failure while doing `operation`.
    pub fn io(operation: impl Into<String>, source: io::Error) -> StepError {
        StepError {
            operation: operation.into(),
            path: None,
            cause: Cause::Io(source),
        }
    }

    /// A git call that failed, with what it said.
    pub fn git(operation: impl Into<String>, said: impl Into<String>) -> StepError {
        StepError {
            operation: operation.into(),
            path: None,
            cause: Cause::Git(GitFailure {
                message: said.into(),
            }),
        }
    }

    /// A refusal: `message` says what is wrong and how to fix it.
    pub fn refused(message: impl Into<String>) -> StepError {
        StepError {
            operation: message.into(),
            path: None,
            cause: Cause::Refused,
        }
    }

    /// Independent failures of one step: the only one itself, else all of
    /// them under `operation`, one per line.
    pub fn several(operation: impl Into<String>, mut errors: Vec<StepError>) -> StepError {
        if errors.len() == 1 {
            return errors.remove(0);
        }
        StepError {
            operation: operation.into(),
            path: None,
            cause: Cause::Several(errors),
        }
    }

    /// The file this failure is about.
    #[must_use]
    pub fn at(mut self, path: &Path) -> StepError {
        self.path = Some(path.to_path_buf());
        self
    }

    pub fn operation(&self) -> &str {
        &self.operation
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

impl fmt::Display for StepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
            Cause::Io(source) => write!(f, "{}: {source}; fix it and run again", self.operation),
            Cause::Git(failure) => write!(f, "{}: {failure}", self.operation),
            Cause::Refused => f.write_str(&self.operation),
            Cause::Several(errors) => {
                write!(f, "{}:", self.operation)?;
                for error in errors {
                    write!(f, "\n  {error}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for StepError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.cause {
            Cause::Io(source) => Some(source),
            Cause::Git(failure) => Some(failure),
            Cause::Refused | Cause::Several(_) => None,
        }
    }
}

/// A step that left something to say: its name and its note.
#[derive(Debug, PartialEq, Eq)]
pub struct StepReport {
    pub name: &'static str,
    pub note: String,
}

impl fmt::Display for StepReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.note)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    AlreadyDone,
    /// The steps that left a note.
    Migrated {
        steps: Vec<StepReport>,
        elapsed: Duration,
    },
    /// The lock is held by another process (hook mode); nothing changed.
    Busy,
    /// A step failed earlier (hook mode); the marker holds what to fix.
    FailedBefore,
    Reversed,
    ReversedAlready,
}

#[derive(Debug)]
pub enum MigrateError {
    NoHome,
    Locked {
        path: PathBuf,
    },
    Io {
        what: String,
        source: io::Error,
    },
    Step {
        step: &'static str,
        source: StepError,
    },
    /// An earlier run failed and left the failed-marker's text.
    FailedBefore {
        note: String,
    },
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrateError::NoHome => write!(f, "no home directory to migrate"),
            MigrateError::Locked { path } => write!(
                f,
                "another process holds the migration lock {}; retry when it finishes",
                path.display()
            ),
            MigrateError::Io { what, source } => write!(f, "{what}: {source}"),
            MigrateError::Step { step, source } => {
                write!(f, "migration step {step} failed: {source}")
            }
            MigrateError::FailedBefore { note } => write!(
                f,
                "an earlier migration failed ({}); run `{} migrate`",
                note.trim().replace('\n', ": "),
                names::BIN
            ),
        }
    }
}

impl std::error::Error for MigrateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MigrateError::Io { source, .. } => Some(source),
            MigrateError::Step { source, .. } => Some(source),
            MigrateError::NoHome
            | MigrateError::Locked { .. }
            | MigrateError::FailedBefore { .. } => None,
        }
    }
}

fn io_error(what: impl Into<String>) -> impl FnOnce(io::Error) -> MigrateError {
    let what = what.into();
    move |source| MigrateError::Io { what, source }
}

/// Whether the migration's done-marker exists.
pub fn done(env: &Env) -> bool {
    done_in(&env.config_dir)
}

/// Whether the done-marker exists in `config_dir`.
pub fn done_in(config_dir: &Path) -> bool {
    exists(&config_dir.join(names::MIGRATE_DONE_MARKER))
}

/// What the markers say, read-only.
pub struct MarkerState {
    /// The done-marker's text (the wall time of the run, in milliseconds).
    pub done: Option<String>,
    pub reversed: bool,
    /// The failed-marker's text.
    pub failed: Option<String>,
}

pub fn marker_state(env: &Env) -> MarkerState {
    MarkerState {
        done: fs::read_to_string(env.marker(names::MIGRATE_DONE_MARKER))
            .ok()
            .map(|text| text.trim().to_string()),
        reversed: exists(&env.marker(names::MIGRATE_REVERSED_MARKER)),
        failed: failure_note(env),
    }
}

/// The old-name state roots that are real directories (not the symlinks the
/// migration leaves behind), read-only.
pub fn old_roots(env: &Env) -> Vec<PathBuf> {
    StateDirectories::roots(env)
        .into_iter()
        .map(|(old_path, _)| old_path)
        .filter(|path| {
            path.symlink_metadata()
                .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
        })
        .collect()
}

/// What the failed-marker says, when one exists.
pub fn failure_note(env: &Env) -> Option<String> {
    fs::read_to_string(env.marker(names::MIGRATE_FAILED_MARKER)).ok()
}

/// Run the real steps forward on this machine.
pub fn machine(wait: Wait) -> Result<Outcome, MigrateError> {
    let env = Env::from_process().ok_or(MigrateError::NoHome)?;
    machine_in(&env, Direction::Forward, wait)
}

/// Run the real steps in `direction` on `env`.
pub fn machine_in(env: &Env, direction: Direction, wait: Wait) -> Result<Outcome, MigrateError> {
    let owned = steps();
    let refs: Vec<&dyn Step> = owned.iter().map(|step| step.as_ref()).collect();
    run(env, direction, wait, &refs)
}

/// The real steps, in forward order.
pub fn steps() -> Vec<Box<dyn Step>> {
    vec![
        Box::new(StateDirectories::new()),
        Box::new(crate::migrate_config::ConfigKeys::new()),
        Box::new(crate::migrate_plugin::PluginRegistration::new()),
    ]
}

/// What the real steps would do in `direction`, each line prefixed with its
/// step's name; reads only and takes no lock.
pub fn machine_plan(env: &Env, direction: Direction) -> Result<Vec<String>, MigrateError> {
    let mut lines = Vec::new();
    let owned = steps();
    let ordered: Vec<&Box<dyn Step>> = match direction {
        Direction::Forward => owned.iter().collect(),
        Direction::Reverse => owned.iter().rev().collect(),
    };
    for step in ordered {
        let planned = step
            .plan(env, direction)
            .map_err(|source| MigrateError::Step {
                step: step.name(),
                source,
            })?;
        lines.extend(
            planned
                .into_iter()
                .map(|line| format!("{}: {line}", step.name())),
        );
    }
    Ok(lines)
}

/// Run the migration for a command that must never fail: every outcome and
/// every error is swallowed. Returns whether a failed-marker is waiting, so a
/// caller that may talk can say the migration is pending.
pub fn preflight() -> bool {
    // `doctor` drives the hook as a probe and promises to write nothing.
    if std::env::var_os(crate::journal::ENV_PROBE).is_some() {
        return false;
    }
    let attempted = std::panic::catch_unwind(|| {
        let Some(env) = Env::from_process() else {
            return false;
        };
        if nothing_old(&env) {
            return failure_note(&env).is_some();
        }
        if let Err(error @ MigrateError::Io { .. }) =
            machine_in(&env, Direction::Forward, Wait::Zero)
        {
            // A lock or marker the run could not use leaves no marker of its
            // own; `doctor` reads this one.
            let _ = fs::write(
                env.marker(names::MIGRATE_FAILED_MARKER),
                format!("migration\n{error}\n"),
            ); // a hook is never failed or blocked by its own bookkeeping
        }
        failure_note(&env).is_some()
    });
    attempted.unwrap_or(false)
}

/// Settle the migration for a command that must not run on a half-moved
/// machine: wait up to `wait` for the lock, retry an earlier failure, and
/// leave a machine the person reversed as it is.
pub fn settle(wait: Duration) -> Result<(), MigrateError> {
    if std::env::var_os(crate::journal::ENV_PROBE).is_some() {
        return Ok(());
    }
    let Some(env) = Env::from_process() else {
        return Ok(());
    };
    if nothing_old(&env) && failure_note(&env).is_none() {
        return Ok(());
    }
    if exists(&env.marker(names::MIGRATE_REVERSED_MARKER)) && !done(&env) {
        return Ok(());
    }
    match machine_in(&env, Direction::Forward, Wait::Capped(wait))? {
        Outcome::Migrated { .. } | Outcome::AlreadyDone | Outcome::ReversedAlready => Ok(()),
        Outcome::Busy => Err(MigrateError::Locked {
            path: env.marker(names::MIGRATE_LOCK_FILE),
        }),
        Outcome::FailedBefore => Err(MigrateError::FailedBefore {
            note: failure_note(&env).unwrap_or_default(),
        }),
        Outcome::Reversed => Ok(()),
    }
}

/// Whether neither old root exists, as a directory or a symlink: a machine
/// that never ran the old tool has nothing to move, and its commands should
/// not pay for the git calls of the config step.
fn nothing_old(env: &Env) -> bool {
    !exists(&env.config_dir.join(old::DATA_DIR))
        && !exists(&env.state_root.with_file_name(old::DATA_DIR))
}

fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

fn remove_marker(path: &Path) -> Result<(), MigrateError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(MigrateError::Io {
            what: format!("cannot remove {}", path.display()),
            source,
        }),
    }
}

fn write_marker(path: &Path, text: &str) -> Result<(), MigrateError> {
    fs::write(path, text).map_err(io_error(format!("cannot write {}", path.display())))
}

pub fn run(
    env: &Env,
    direction: Direction,
    wait: Wait,
    steps: &[&dyn Step],
) -> Result<Outcome, MigrateError> {
    let started = Instant::now();
    let done = env.marker(names::MIGRATE_DONE_MARKER);
    let reversed = env.marker(names::MIGRATE_REVERSED_MARKER);
    let failed = env.marker(names::MIGRATE_FAILED_MARKER);

    if direction == Direction::Forward && exists(&done) && !exists(&reversed) {
        return Ok(Outcome::AlreadyDone);
    }
    fs::create_dir_all(&env.config_dir).map_err(io_error(format!(
        "cannot create {}",
        env.config_dir.display()
    )))?;
    let lock_path = env.marker(names::MIGRATE_LOCK_FILE);
    let budget = match wait {
        Wait::Zero => Duration::ZERO,
        Wait::Capped(cap) => cap,
    };
    let held = lock_within(&lock_path, budget)
        .map_err(io_error(format!("cannot take {}", lock_path.display())))?;
    let Some(_held) = held else {
        return match wait {
            Wait::Zero => Ok(Outcome::Busy),
            Wait::Capped(_) => Err(MigrateError::Locked { path: lock_path }),
        };
    };

    match direction {
        Direction::Forward => {
            match wait {
                Wait::Zero => {
                    if exists(&reversed) {
                        return Ok(Outcome::ReversedAlready);
                    }
                    if exists(&failed) {
                        return Ok(Outcome::FailedBefore);
                    }
                }
                Wait::Capped(_) => {
                    remove_marker(&reversed)?;
                    remove_marker(&failed)?;
                }
            }
            if exists(&done) {
                return Ok(Outcome::AlreadyDone);
            }
            let mut reports = Vec::new();
            for step in steps {
                match step.forward(env) {
                    Ok(note) if note.is_empty() => {}
                    Ok(note) => reports.push(StepReport {
                        name: step.name(),
                        note,
                    }),
                    Err(source) => {
                        write_marker(&failed, &format!("{}\n{source}\n", step.name()))?;
                        return Err(MigrateError::Step {
                            step: step.name(),
                            source,
                        });
                    }
                }
            }
            write_marker(&done, &format!("{}\n", started.elapsed().as_millis()))?;
            remove_marker(&reversed)?;
            remove_marker(&failed)?;
            Ok(Outcome::Migrated {
                steps: reports,
                elapsed: started.elapsed(),
            })
        }
        Direction::Reverse => {
            if exists(&reversed) && !exists(&done) {
                return Ok(Outcome::ReversedAlready);
            }
            for step in steps.iter().rev() {
                step.reverse(env).map_err(|source| MigrateError::Step {
                    step: step.name(),
                    source,
                })?;
            }
            write_marker(&reversed, "")?;
            remove_marker(&done)?;
            remove_marker(&failed)?;
            Ok(Outcome::Reversed)
        }
    }
}

/// The state-directory step: both roots move by rename, and a symlink stays
/// at the old path.
pub struct StateDirectories {
    after_rename: fn(&Path),
    clock_seconds: fn() -> u64,
}

fn no_hook(_old_path: &Path) {}

fn system_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn is_log_name(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    name == "journal.log" || name.ends_with(".jsonl")
}

/// What a log file starts with.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LogShape {
    /// A header line, then rows (`journal.log`).
    Journal,
    /// Rows only (`*.jsonl`).
    Rows,
}

impl LogShape {
    fn of(name: &std::ffi::OsStr) -> LogShape {
        if name == "journal.log" {
            LogShape::Journal
        } else {
            LogShape::Rows
        }
    }
}

/// The suffix of the temp file a rewrite goes through.
const TEMP_SUFFIX: &str = ".migrate-tmp";

fn is_temp_name(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().ends_with(TEMP_SUFFIX)
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(TEMP_SUFFIX);
    path.with_file_name(name)
}

#[cfg(unix)]
fn create_temp(temp: &Path, original: &fs::Permissions) -> io::Result<fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(original.mode())
        .open(temp)
}

#[cfg(not(unix))]
fn create_temp(temp: &Path, _original: &fs::Permissions) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)
}

fn write_temp(temp: &Path, original: &fs::Permissions, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut file = match create_temp(temp, original) {
        Ok(file) => file,
        // A leftover of a crashed run: the name is ours, so it is replaced.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_file(temp)?;
            create_temp(temp, original)?
        }
        Err(e) => return Err(e),
    };
    file.set_permissions(original.clone())?; // the process umask must not narrow the mode
    file.write_all(bytes)?;
    file.sync_all()
}

/// Replace `path` with `bytes` through a new temp file that has the original's
/// mode, is synced, and is renamed over it; the temp file is removed on any
/// error.
fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = temp_path(path);
    let original = fs::metadata(path)?.permissions();
    let outcome = write_temp(&temp, &original, bytes).and_then(|()| fs::rename(&temp, path));
    if outcome.is_err() {
        let _ = fs::remove_file(&temp); // the write's own error is the one to report
    }
    outcome
}

/// Merge the old file's rows ahead of the new file's into `new_file`, through
/// a temp file renamed over it with the mode of the file it replaces. A
/// journal keeps the new file's header line once; each file's own first line
/// is dropped from the rows.
fn append_merge(old_file: &Path, new_file: &Path, shape: LogShape) -> io::Result<()> {
    let old_bytes = fs::read(old_file)?;
    let new_bytes = fs::read(new_file)?;
    let mut out = Vec::with_capacity(old_bytes.len() + new_bytes.len() + 1);
    let (old_rows, new_rows, header): (&[u8], &[u8], &[u8]) = match shape {
        LogShape::Journal => {
            let (_, old_rest) = split_first_line(&old_bytes);
            let (new_header, new_rest) = split_first_line(&new_bytes);
            (old_rest, new_rest, new_header)
        }
        LogShape::Rows => (&old_bytes, &new_bytes, &[]),
    };
    out.extend_from_slice(header);
    if !header.is_empty() && !header.ends_with(b"\n") {
        out.push(b'\n');
    }
    out.extend_from_slice(old_rows);
    if !old_rows.is_empty() && !old_rows.ends_with(b"\n") {
        out.push(b'\n');
    }
    out.extend_from_slice(new_rows);
    replace_file(new_file, &out)
}

fn split_first_line(bytes: &[u8]) -> (&[u8], &[u8]) {
    match bytes.iter().position(|byte| *byte == b'\n') {
        Some(end) => bytes.split_at(end + 1),
        None => (bytes, &[]),
    }
}

impl StateDirectories {
    pub fn new() -> StateDirectories {
        StateDirectories {
            after_rename: no_hook,
            clock_seconds: system_seconds,
        }
    }

    /// A step that calls `hook` with the old path between the rename and the
    /// symlink, for a test that races a recreated old directory.
    pub fn with_hook_after_rename(hook: fn(&Path)) -> StateDirectories {
        StateDirectories {
            after_rename: hook,
            ..StateDirectories::new()
        }
    }

    /// The (old, new) pairs: the config-side root and the state-side root.
    fn roots(env: &Env) -> Vec<(PathBuf, PathBuf)> {
        let mut roots = vec![(
            env.config_dir.join(old::DATA_DIR),
            env.config_dir.join(names::DATA_DIR),
        )];
        if let Some(parent) = env.state_root.parent() {
            roots.push((parent.join(old::DATA_DIR), env.state_root.clone()));
        }
        roots
    }

    /// The note says what moved, "" when the old path was already settled.
    fn move_forward(&self, old_path: &Path, new_path: &Path) -> Result<String, StepError> {
        let old_is_dir = old_path
            .symlink_metadata()
            .ok()
            .is_some_and(|meta| meta.is_dir());
        let new_exists = exists(new_path);
        let mut did = "";
        if old_is_dir && !new_exists {
            fs::rename(old_path, new_path).map_err(|e| {
                StepError::io(
                    format!(
                        "cannot rename {} to {}",
                        old_path.display(),
                        new_path.display()
                    ),
                    e,
                )
                .at(old_path)
            })?;
            did = "moved";
            (self.after_rename)(old_path);
        } else if !exists(old_path) && !new_exists {
            return Ok(String::new());
        }
        let linked = match old_path.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => String::new(),
            Ok(meta) if meta.is_dir() => {
                self.absorb(old_path, new_path)?;
                did = "merged";
                link(old_path, new_path).map_err(|e| link_error(old_path, new_path, e))?
            }
            Ok(_) => {
                return Err(StepError::refused(format!(
                    "{} is a file, not a directory; move it away and run again",
                    old_path.display()
                ))
                .at(old_path))
            }
            Err(_) => {
                did = if did.is_empty() { "linked" } else { did };
                link(old_path, new_path).map_err(|e| link_error(old_path, new_path, e))?
            }
        };
        let moved = if did.is_empty() {
            String::new()
        } else {
            format!("{did} {} to {}", old_path.display(), new_path.display())
        };
        Ok(match (moved.is_empty(), linked.is_empty()) {
            (_, true) => moved,
            (true, false) => linked,
            (false, false) => format!("{moved}; {linked}"),
        })
    }

    /// Merge the directory `from` into `into`, then remove it.
    fn absorb(&self, from: &Path, into: &Path) -> Result<(), StepError> {
        self.merge(from, into)
            .map_err(|e| StepError::io(format!("cannot merge {}", from.display()), e).at(from))?;
        fs::remove_dir(from)
            .map_err(|e| StepError::io(format!("cannot remove {}", from.display()), e).at(from))
    }

    /// Move every entry of `from` into `into`. Two regular log files
    /// (`journal.log`, `*.jsonl`) at the same name are appended (old rows
    /// first); two directories are merged recursively; any other entry that
    /// exists in both stays as the one in `into`, and the other lands beside
    /// it.
    fn merge(&self, from: &Path, into: &Path) -> io::Result<()> {
        let seconds = (self.clock_seconds)();
        for entry in fs::read_dir(from)? {
            let name = entry?.file_name();
            let source = from.join(&name);
            let mut target = into.join(&name);
            if exists(&target) {
                let source_meta = source.symlink_metadata()?;
                let target_meta = target.symlink_metadata()?;
                if source_meta.is_dir()
                    && target_meta.is_dir()
                    && !source_meta.file_type().is_symlink()
                    && !target_meta.file_type().is_symlink()
                {
                    self.merge(&source, &target)?;
                    fs::remove_dir(&source)?;
                    continue;
                }
                if source_meta.is_file() && target_meta.is_file() && is_log_name(&name) {
                    append_merge(&source, &target, LogShape::of(&name))?;
                    fs::remove_file(&source)?;
                    continue;
                }
            }
            let mut attempt = 0u32;
            while exists(&target) {
                let suffix = if attempt == 0 {
                    String::new()
                } else {
                    format!("-{attempt}")
                };
                target = into.join(format!(
                    "{}.from-old-{seconds}{suffix}",
                    name.to_string_lossy()
                ));
                attempt += 1;
            }
            fs::rename(&source, &target)?;
        }
        Ok(())
    }

    fn move_back(&self, old_path: &Path, new_path: &Path) -> Result<(), StepError> {
        match old_path.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => {
                fs::remove_file(old_path).map_err(|e| {
                    StepError::io(format!("cannot remove {}", old_path.display()), e).at(old_path)
                })?
            }
            Ok(meta) if meta.is_dir() && exists(new_path) => self.absorb(old_path, new_path)?,
            _ => {}
        }
        if exists(new_path) && !exists(old_path) {
            fs::rename(new_path, old_path).map_err(|e| {
                StepError::io(
                    format!(
                        "cannot rename {} to {}",
                        new_path.display(),
                        old_path.display()
                    ),
                    e,
                )
                .at(new_path)
            })?;
        }
        Ok(())
    }
}

impl Default for StateDirectories {
    fn default() -> Self {
        StateDirectories::new()
    }
}

fn link_error(old_path: &Path, new_path: &Path, source: io::Error) -> StepError {
    StepError::io(
        format!(
            "cannot link {} to {}",
            old_path.display(),
            new_path.display()
        ),
        source,
    )
    .at(old_path)
}

#[cfg(unix)]
fn link(old_path: &Path, new_path: &Path) -> io::Result<String> {
    std::os::unix::fs::symlink(new_path, old_path)?;
    Ok(String::new())
}

#[cfg(not(unix))]
fn link(_old_path: &Path, _new_path: &Path) -> io::Result<String> {
    Ok("no symlink left at the old path on this platform".to_string())
}

impl Step for StateDirectories {
    fn name(&self) -> &'static str {
        "state directories"
    }

    fn forward(&self, env: &Env) -> Result<String, StepError> {
        let mut notes = Vec::new();
        for (old_path, new_path) in StateDirectories::roots(env) {
            let note = self.move_forward(&old_path, &new_path)?;
            if !note.is_empty() {
                notes.push(note);
            }
        }
        Ok(notes.join("; "))
    }

    fn plan(&self, env: &Env, direction: Direction) -> Result<Vec<String>, StepError> {
        let mut lines = Vec::new();
        for (old_path, new_path) in StateDirectories::roots(env) {
            let (old_meta, new_exists) = (old_path.symlink_metadata().ok(), exists(&new_path));
            let (old, new) = (old_path.display(), new_path.display());
            match direction {
                Direction::Forward => match old_meta {
                    Some(meta) if meta.file_type().is_symlink() => {}
                    Some(meta) if meta.is_dir() && new_exists => {
                        lines.push(format!(
                            "merge {old} into {new} and leave a symlink at {old}"
                        ));
                    }
                    Some(meta) if meta.is_dir() => {
                        lines.push(format!(
                            "rename {old} to {new} and leave a symlink at {old}"
                        ));
                    }
                    Some(_) => lines.push(format!("refuse: {old} is a file, not a directory")),
                    None if new_exists => lines.push(format!("leave a symlink at {old} to {new}")),
                    None => {}
                },
                Direction::Reverse => {
                    match &old_meta {
                        Some(meta) if meta.file_type().is_symlink() => {
                            lines.push(format!("remove the symlink at {old}"));
                        }
                        Some(meta) if meta.is_dir() && new_exists => {
                            lines.push(format!("merge {old} into {new}"));
                        }
                        _ => {}
                    }
                    if new_exists {
                        lines.push(format!("rename {new} to {old}"));
                    }
                }
            }
        }
        if direction == Direction::Reverse && exists(&env.config_dir.join(names::DATA_DIR)) {
            lines.push(format!(
                "rewrite the journal header and review-row schemas under {} to the old names",
                env.config_dir.join(old::DATA_DIR).display()
            ));
        }
        Ok(lines)
    }

    fn reverse(&self, env: &Env) -> Result<String, StepError> {
        for (old_path, new_path) in StateDirectories::roots(env) {
            self.move_back(&old_path, &new_path)?;
        }
        let config_old = env.config_dir.join(old::DATA_DIR);
        let is_real_dir = config_old
            .symlink_metadata()
            .ok()
            .is_some_and(|meta| meta.is_dir());
        if is_real_dir {
            rewrite_rows_back(&config_old).map_err(|e| {
                StepError::io(
                    format!("cannot rewrite the rows under {}", config_old.display()),
                    e,
                )
                .at(&config_old)
            })?;
        }
        Ok(String::new())
    }
}

const SCHEMAS: [(&str, &str); 4] = [
    (names::REVIEW_OUTCOME_SCHEMA, old::REVIEW_OUTCOME_SCHEMA),
    (names::REVIEW_MERGE_SCHEMA, old::REVIEW_MERGE_SCHEMA),
    (
        names::REVIEW_DISPOSITION_SCHEMA,
        old::REVIEW_DISPOSITION_SCHEMA,
    ),
    (
        names::IMPL_REVIEW_OUTCOME_SCHEMA,
        old::IMPL_REVIEW_OUTCOME_SCHEMA,
    ),
];

/// The line with its `schema` value renamed to the old schema, when it is a
/// JSON row of a new review schema; every other byte is kept.
fn row_back(line: &str) -> Option<String> {
    for (new_schema, old_schema) in SCHEMAS {
        if !line.contains(new_schema) {
            continue;
        }
        let row: serde_json::Value = serde_json::from_str(line).ok()?;
        if row.get("schema").and_then(|schema| schema.as_str()) != Some(new_schema) {
            continue;
        }
        let key = line.find("\"schema\"")?;
        let quoted = format!("\"{new_schema}\"");
        let at = key + line[key..].find(&quoted)?;
        return Some(format!(
            "{}\"{old_schema}\"{}",
            &line[..at],
            &line[at + quoted.len()..]
        ));
    }
    None
}

fn rewrite_rows_back(config_old: &Path) -> io::Result<()> {
    for entry in fs::read_dir(config_old)? {
        let entry = entry?;
        let name = entry.file_name();
        let is_journal = name.to_string_lossy().starts_with("journal.log");
        if is_journal && !is_temp_name(&name) && entry.file_type()?.is_file() {
            rewrite_file(&entry.path(), LogShape::Journal)?;
        }
    }
    let plan_review = config_old.join("plan-review");
    if plan_review.is_dir() {
        rewrite_jsonl_under(&plan_review)?;
    }
    Ok(())
}

fn rewrite_jsonl_under(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            rewrite_jsonl_under(&entry.path())?;
        } else if kind.is_file()
            && !is_temp_name(&entry.file_name())
            && entry.path().extension().is_some_and(|ext| ext == "jsonl")
        {
            rewrite_file(&entry.path(), LogShape::Rows)?;
        }
    }
    Ok(())
}

/// Rewrite the rows of one file through [`replace_file`]; a file with nothing
/// to change is not touched.
fn rewrite_file(path: &Path, shape: LogShape) -> io::Result<()> {
    let bytes = fs::read(path)?;
    let mut out = Vec::with_capacity(bytes.len());
    let mut changed = false;
    for (index, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let body_len = line.strip_suffix(b"\n").map_or(line.len(), <[u8]>::len);
        let rewritten = std::str::from_utf8(&line[..body_len])
            .ok()
            .and_then(|text| {
                if shape == LogShape::Journal && index == 0 && text == names::JOURNAL_HEADER {
                    Some(old::JOURNAL_HEADER.to_string())
                } else {
                    row_back(text)
                }
            });
        match rewritten {
            Some(text) => {
                changed = true;
                out.extend_from_slice(text.as_bytes());
                out.extend_from_slice(&line[body_len..]);
            }
            None => out.extend_from_slice(line),
        }
    }
    if !changed {
        return Ok(());
    }
    replace_file(path, &out)
}

/// Tests that hold the migration lock while another spawns git would share
/// the lock's descriptor through the fork; one at a time avoids that.
#[cfg(all(test, unix))]
pub(crate) fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct Scratch(PathBuf, std::sync::MutexGuard<'static, ()>);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _serial = &self.1; // the guard outlives the cleanup
            let _ = fs::remove_dir_all(&self.0); // best-effort cleanup of a temp dir
        }
    }

    fn schema_row(schema: &str, number: u32) -> String {
        format!("{{\"schema\":\"{schema}\",\"n\":{number}}}\n")
    }

    fn old_rows() -> String {
        [
            old::REVIEW_OUTCOME_SCHEMA,
            old::REVIEW_MERGE_SCHEMA,
            old::REVIEW_DISPOSITION_SCHEMA,
            old::IMPL_REVIEW_OUTCOME_SCHEMA,
        ]
        .iter()
        .zip(0u32..)
        .map(|(schema, number)| schema_row(schema, number))
        .collect()
    }

    /// A scratch machine: old-named config and state roots.
    fn machine_tree() -> (Scratch, Env) {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("gp-migrate-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&root); // a leftover of a crashed run
        let config_dir = root.join("claude");
        let state = root.join("state");
        let config_old = config_dir.join(old::DATA_DIR);
        fs::create_dir_all(config_old.join("plan-review")).unwrap();
        fs::create_dir_all(config_old.join("attestations")).unwrap();
        fs::create_dir_all(config_old.join("provenance")).unwrap();
        fs::write(
            config_old.join("journal.log"),
            format!("{}\nrow one\n", old::JOURNAL_HEADER),
        )
        .unwrap();
        fs::write(config_old.join("plan-review/outcomes.jsonl"), old_rows()).unwrap();
        fs::write(config_old.join("attestations/a1"), "attest").unwrap();
        fs::write(config_old.join("provenance/p1"), "prov").unwrap();
        let marketplace = state.join(old::DATA_DIR).join("claude-marketplace");
        fs::create_dir_all(&marketplace).unwrap();
        fs::write(marketplace.join("m.json"), "{}").unwrap();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let scratch_env =
            |name: &str, value: PathBuf| (OsString::from(name), value.into_os_string());
        let env = Env {
            config_dir,
            state_root: state.join(names::DATA_DIR),
            git_env: Some(vec![
                scratch_env("HOME", home.clone()),
                scratch_env("XDG_CONFIG_HOME", root.join("xdg")),
                scratch_env("GIT_CONFIG_GLOBAL", home.join(".gitconfig")),
                scratch_env("GIT_CONFIG_SYSTEM", root.join("system.gitconfig")),
                (OsString::from("GIT_CONFIG_NOSYSTEM"), OsString::from("1")),
            ]),
        };
        (Scratch(root, crate::migrate::serial()), env)
    }

    fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        fn walk(base: &Path, dir: &Path) -> BTreeMap<String, Vec<u8>> {
            let mut found = BTreeMap::new();
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let key = path.strip_prefix(base).unwrap().display().to_string();
                if path.is_dir() {
                    found.insert(format!("{key}/"), Vec::new());
                    found.extend(walk(base, &path));
                } else {
                    found.insert(key, fs::read(&path).unwrap());
                }
            }
            found
        }
        walk(dir, dir)
    }

    fn mtimes(dir: &Path) -> BTreeMap<PathBuf, SystemTime> {
        let mut found = BTreeMap::new();
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            if path.is_dir() {
                found.extend(mtimes(&path));
            }
            found.insert(path, modified);
        }
        found
    }

    fn zero(env: &Env, direction: Direction) -> Result<Outcome, MigrateError> {
        machine_in(env, direction, Wait::Zero)
    }

    fn explicit(env: &Env) -> Result<Outcome, MigrateError> {
        machine_in(
            env,
            Direction::Forward,
            Wait::Capped(Duration::from_secs(1)),
        )
    }

    fn config_old(env: &Env) -> PathBuf {
        env.config_dir.join(old::DATA_DIR)
    }

    fn config_new(env: &Env) -> PathBuf {
        env.config_dir.join(names::DATA_DIR)
    }

    fn state_old(env: &Env) -> PathBuf {
        env.state_root.parent().unwrap().join(old::DATA_DIR)
    }

    #[test]
    fn forward_moves_both_roots_and_leaves_symlinks() {
        let (_scratch, env) = machine_tree();
        let before = snapshot(&config_old(&env));
        let state_before = snapshot(&state_old(&env));
        let Outcome::Migrated { steps, elapsed } = zero(&env, Direction::Forward).unwrap() else {
            panic!("not migrated");
        };
        let named: Vec<&str> = steps.iter().map(|report| report.name).collect();
        assert_eq!(named, vec!["state directories"], "{steps:?}");
        assert!(steps[0].note.contains("moved"), "{steps:?}");
        assert!(elapsed < Duration::from_secs(60));
        assert_eq!(snapshot(&config_new(&env)), before);
        assert_eq!(snapshot(&env.state_root), state_before);
        assert_eq!(fs::read_link(config_old(&env)).unwrap(), config_new(&env));
        assert_eq!(fs::read_link(state_old(&env)).unwrap(), env.state_root);
        assert!(exists(&env.config_dir.join(names::MIGRATE_DONE_MARKER)));
    }

    #[test]
    fn plan_reads_only() {
        let (_scratch, env) = machine_tree();
        let step = StateDirectories::new();
        for direction in [Direction::Forward, Direction::Reverse] {
            let config = snapshot(&env.config_dir);
            let state = snapshot(state_old(&env).parent().unwrap());
            let times = mtimes(&env.config_dir);
            let lines = step.plan(&env, direction).unwrap();
            if direction == Direction::Forward {
                assert_eq!(lines.len(), 2, "{lines:?}");
            }
            assert_eq!(snapshot(&env.config_dir), config);
            assert_eq!(snapshot(state_old(&env).parent().unwrap()), state);
            assert_eq!(mtimes(&env.config_dir), times);
        }
    }

    #[test]
    fn a_second_forward_changes_nothing() {
        let (_scratch, env) = machine_tree();
        zero(&env, Direction::Forward).unwrap();
        let contents = snapshot(&config_new(&env));
        let times = mtimes(&config_new(&env));
        assert_eq!(
            zero(&env, Direction::Forward).unwrap(),
            Outcome::AlreadyDone
        );
        assert_eq!(explicit(&env).unwrap(), Outcome::AlreadyDone);
        assert_eq!(snapshot(&config_new(&env)), contents);
        assert_eq!(mtimes(&config_new(&env)), times);
    }

    #[test]
    fn reverse_restores_the_tree_and_rewrites_new_rows() {
        let (_scratch, env) = machine_tree();
        let before = snapshot(&config_old(&env));
        let state_before = snapshot(&state_old(&env));
        zero(&env, Direction::Forward).unwrap();
        let journal = config_new(&env).join("journal.log");
        let journal_text = fs::read_to_string(&journal).unwrap();
        let rest = journal_text.split_once('\n').unwrap().1;
        fs::write(
            &journal,
            format!("{}\n{rest}row from five\n", names::JOURNAL_HEADER),
        )
        .unwrap();
        let outcomes = config_new(&env).join("plan-review/outcomes.jsonl");
        let mut text = fs::read_to_string(&outcomes).unwrap();
        text.push_str(&schema_row(names::REVIEW_OUTCOME_SCHEMA, 9));
        fs::write(&outcomes, text).unwrap();

        assert_eq!(zero(&env, Direction::Reverse).unwrap(), Outcome::Reversed);

        let mut expected = before;
        expected
            .get_mut("journal.log")
            .unwrap()
            .extend_from_slice(b"row from five\n");
        expected
            .get_mut("plan-review/outcomes.jsonl")
            .unwrap()
            .extend_from_slice(schema_row(old::REVIEW_OUTCOME_SCHEMA, 9).as_bytes());
        assert_eq!(snapshot(&config_old(&env)), expected);
        assert_eq!(snapshot(&state_old(&env)), state_before);
        for gone in [
            config_new(&env),
            config_new(&env).join("plan-review/outcomes.jsonl"),
            config_new(&env).join("provenance"),
            env.state_root.clone(),
        ] {
            assert_eq!(
                gone.symlink_metadata().unwrap_err().kind(),
                io::ErrorKind::NotFound,
                "{}",
                gone.display()
            );
        }
        assert!(!exists(&env.config_dir.join(names::MIGRATE_DONE_MARKER)));
        assert!(exists(&env.config_dir.join(names::MIGRATE_REVERSED_MARKER)));
        assert_eq!(
            zero(&env, Direction::Reverse).unwrap(),
            Outcome::ReversedAlready
        );
    }

    #[test]
    fn a_hook_never_migrates_after_a_reverse_but_an_explicit_run_does() {
        let (_scratch, env) = machine_tree();
        zero(&env, Direction::Forward).unwrap();
        zero(&env, Direction::Reverse).unwrap();
        assert_eq!(
            zero(&env, Direction::Forward).unwrap(),
            Outcome::ReversedAlready
        );
        assert!(!exists(&config_new(&env)));
        assert!(matches!(explicit(&env).unwrap(), Outcome::Migrated { .. }));
        assert!(!exists(
            &env.config_dir.join(names::MIGRATE_REVERSED_MARKER)
        ));
        assert!(config_new(&env).is_dir());
    }

    #[test]
    fn an_interrupted_rename_resumes_with_the_symlink() {
        let (_scratch, env) = machine_tree();
        fs::rename(config_old(&env), config_new(&env)).unwrap();
        assert!(matches!(
            zero(&env, Direction::Forward).unwrap(),
            Outcome::Migrated { .. }
        ));
        assert_eq!(fs::read_link(config_old(&env)).unwrap(), config_new(&env));
        assert!(env.state_root.is_dir());
    }

    #[test]
    fn a_missing_old_root_is_done() {
        let (_scratch, env) = machine_tree();
        fs::remove_dir_all(state_old(&env)).unwrap();
        assert!(matches!(
            zero(&env, Direction::Forward).unwrap(),
            Outcome::Migrated { .. }
        ));
        assert!(!exists(&env.state_root));
    }

    fn recreate_old(old_path: &Path) {
        if old_path.ends_with(old::DATA_DIR) && old_path.parent().unwrap().ends_with("claude") {
            fs::create_dir_all(old_path).unwrap();
            fs::write(old_path.join("fresh"), "fresh").unwrap();
            fs::write(old_path.join("colliding"), "old copy").unwrap();
        }
    }

    #[test]
    fn a_recreated_old_directory_is_merged_before_the_symlink() {
        let (_scratch, env) = machine_tree();
        fs::write(config_old(&env).join("colliding"), "new copy").unwrap();
        let step = StateDirectories::with_hook_after_rename(recreate_old);
        let only: [&dyn Step; 1] = [&step];
        let outcome = run(&env, Direction::Forward, Wait::Zero, &only).unwrap();
        assert!(matches!(outcome, Outcome::Migrated { .. }));
        let new = config_new(&env);
        assert_eq!(fs::read_to_string(new.join("fresh")).unwrap(), "fresh");
        assert_eq!(
            fs::read_to_string(new.join("colliding")).unwrap(),
            "new copy"
        );
        let beside: Vec<String> = fs::read_dir(&new)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("colliding.from-old-"))
            .collect();
        assert_eq!(beside.len(), 1);
        assert_eq!(
            fs::read_to_string(new.join(&beside[0])).unwrap(),
            "old copy"
        );
        assert_eq!(fs::read_link(config_old(&env)).unwrap(), new);
    }

    fn scratch_dir(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "gp-merge-{}-{tag}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir); // a leftover of a crashed run
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir, serial())
    }

    #[test]
    fn colliding_logs_are_appended_old_rows_first() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = scratch_dir("logs");
        let (from, into) = (scratch.0.join("from"), scratch.0.join("into"));
        fs::create_dir_all(from.join("plan-review")).unwrap();
        fs::create_dir_all(into.join("plan-review")).unwrap();
        fs::write(from.join("journal.log"), "old-header\nold1\nold2\n").unwrap();
        fs::write(into.join("journal.log"), "new-header\nnew1\n").unwrap();
        fs::set_permissions(into.join("journal.log"), fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(from.join("plan-review/o.jsonl"), "a\nb").unwrap();
        fs::write(into.join("plan-review/o.jsonl"), "c\n").unwrap();
        StateDirectories::new().absorb(&from, &into).unwrap();
        assert_eq!(
            fs::read_to_string(into.join("journal.log")).unwrap(),
            "new-header\nold1\nold2\nnew1\n"
        );
        assert_eq!(
            fs::metadata(into.join("journal.log"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_to_string(into.join("plan-review/o.jsonl")).unwrap(),
            "a\nb\nc\n"
        );
        assert!(!from.exists());
        let leftovers = fs::read_dir(&into)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("from-old")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn other_collisions_keep_a_side_copy() {
        let scratch = scratch_dir("other");
        let (from, into) = (scratch.0.join("from"), scratch.0.join("into"));
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&into).unwrap();
        fs::write(from.join("notes.txt"), "old").unwrap();
        fs::write(into.join("notes.txt"), "new").unwrap();
        StateDirectories::new().absorb(&from, &into).unwrap();
        assert_eq!(fs::read_to_string(into.join("notes.txt")).unwrap(), "new");
        let side: Vec<_> = fs::read_dir(&into)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("notes.txt.from-old-"))
            .collect();
        assert_eq!(side.len(), 1);
        assert_eq!(fs::read_to_string(into.join(&side[0])).unwrap(), "old");
    }

    #[test]
    fn colliding_directories_merge_recursively() {
        let scratch = scratch_dir("dirs");
        let (from, into) = (scratch.0.join("from"), scratch.0.join("into"));
        fs::create_dir_all(from.join("sub")).unwrap();
        fs::create_dir_all(into.join("sub")).unwrap();
        fs::write(from.join("sub/only-old"), "o").unwrap();
        fs::write(into.join("sub/only-new"), "n").unwrap();
        fs::write(from.join("sub/same"), "old").unwrap();
        fs::write(into.join("sub/same"), "new").unwrap();
        StateDirectories::new().absorb(&from, &into).unwrap();
        assert_eq!(fs::read_to_string(into.join("sub/only-old")).unwrap(), "o");
        assert_eq!(fs::read_to_string(into.join("sub/only-new")).unwrap(), "n");
        assert_eq!(fs::read_to_string(into.join("sub/same")).unwrap(), "new");
        let side = fs::read_dir(into.join("sub"))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("same.from-old-")
            })
            .count();
        assert_eq!(side, 1);
        assert!(!into.join("sub.from-old-0").exists());
    }

    #[test]
    fn a_held_lock_is_busy_for_a_hook_and_locked_for_a_capped_wait() {
        let (_scratch, env) = machine_tree();
        fs::create_dir_all(&env.config_dir).unwrap();
        let lock_path = env.config_dir.join(names::MIGRATE_LOCK_FILE);
        let held = lock_within(&lock_path, Duration::ZERO).unwrap().unwrap();
        let before = snapshot(&config_old(&env));
        assert_eq!(zero(&env, Direction::Forward).unwrap(), Outcome::Busy);
        let capped = Wait::Capped(Duration::from_millis(100));
        match machine_in(&env, Direction::Forward, capped).unwrap_err() {
            MigrateError::Locked { path } => assert_eq!(path, lock_path),
            other => panic!("{other}"),
        }
        assert_eq!(snapshot(&config_old(&env)), before);
        assert!(!exists(&config_new(&env)));
        drop(held);
    }

    struct Flaky {
        attempts: Cell<usize>,
    }

    impl Step for Flaky {
        fn name(&self) -> &'static str {
            "flaky"
        }
        fn forward(&self, _env: &Env) -> Result<String, StepError> {
            self.attempts.set(self.attempts.get() + 1);
            if self.attempts.get() == 1 {
                Err(StepError::io(
                    "cannot write the disk",
                    io::Error::other("disk says no"),
                ))
            } else {
                Ok(String::new())
            }
        }
        fn reverse(&self, _env: &Env) -> Result<String, StepError> {
            Ok(String::new())
        }
        fn plan(&self, _env: &Env, _direction: Direction) -> Result<Vec<String>, StepError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn a_failure_is_attempted_once_by_hooks_and_retried_by_an_explicit_run() {
        let (_scratch, env) = machine_tree();
        let flaky = Flaky {
            attempts: Cell::new(0),
        };
        let state = StateDirectories::new();
        let both: [&dyn Step; 2] = [&state, &flaky];
        let first = run(&env, Direction::Forward, Wait::Zero, &both).unwrap_err();
        assert!(first.to_string().contains("disk says no"));
        let note = failure_note(&env).unwrap();
        assert!(note.starts_with("flaky\n") && note.contains("disk says no"));
        assert!(config_new(&env).is_dir(), "earlier steps' results stay");
        for _ in 0..49 {
            let outcome = run(&env, Direction::Forward, Wait::Zero, &both).unwrap();
            assert_eq!(outcome, Outcome::FailedBefore);
        }
        assert_eq!(flaky.attempts.get(), 1);
        let capped = Wait::Capped(Duration::from_secs(1));
        let retried = run(&env, Direction::Forward, capped, &both).unwrap();
        assert!(matches!(retried, Outcome::Migrated { .. }));
        assert_eq!(flaky.attempts.get(), 2);
        assert!(failure_note(&env).is_none());
        assert!(exists(&env.config_dir.join(names::MIGRATE_DONE_MARKER)));
    }

    #[test]
    fn an_io_failure_keeps_its_source_and_the_repair_text() {
        let (_scratch, env) = machine_tree();
        let flaky = Flaky {
            attempts: Cell::new(0),
        };
        let only: [&dyn Step; 1] = [&flaky];
        let error = run(&env, Direction::Forward, Wait::Zero, &only).unwrap_err();
        let MigrateError::Step { step, source } = &error else {
            panic!("{error}");
        };
        assert_eq!(*step, "flaky");
        let cause = std::error::Error::source(source).expect("the I/O error is the source");
        assert!(cause.to_string().contains("disk says no"));
        assert!(
            source.to_string().ends_with("fix it and run again"),
            "{source}"
        );
        assert!(std::error::Error::source(&error).is_some());
    }

    fn journal_with_new_rows(dir: &Path) -> PathBuf {
        let journal = dir.join("journal.log");
        fs::write(&journal, format!("{}\nrow\n", names::JOURNAL_HEADER)).unwrap();
        journal
    }

    #[test]
    fn the_row_rewrite_keeps_the_mode_and_leaves_no_temp_file() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = scratch_dir("rewrite");
        let journal = journal_with_new_rows(&scratch.0);
        fs::set_permissions(&journal, fs::Permissions::from_mode(0o600)).unwrap();
        rewrite_file(&journal, LogShape::Journal).unwrap();
        assert_eq!(
            fs::read_to_string(&journal).unwrap(),
            format!("{}\nrow\n", old::JOURNAL_HEADER)
        );
        assert_eq!(
            fs::metadata(&journal).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!temp_path(&journal).exists());
    }

    #[test]
    fn a_failing_rewrite_removes_its_temp_file() {
        let scratch = scratch_dir("rewrite-fails");
        let target = scratch.0.join("target");
        fs::create_dir(&target).unwrap();
        assert!(replace_file(&target, b"rows\n").is_err());
        assert!(!temp_path(&target).exists());
    }

    #[test]
    fn the_row_walk_ignores_a_leftover_temp_file() {
        let scratch = scratch_dir("leftover");
        let leftover = scratch.0.join("journal.log.migrate-tmp");
        let text = format!("{}\nstale\n", names::JOURNAL_HEADER);
        fs::write(&leftover, &text).unwrap();
        fs::create_dir(scratch.0.join("plan-review")).unwrap();
        let ledger_leftover = scratch.0.join("plan-review/o.jsonl.migrate-tmp");
        let ledger_text = schema_row(names::REVIEW_OUTCOME_SCHEMA, 1);
        fs::write(&ledger_leftover, &ledger_text).unwrap();
        rewrite_rows_back(&scratch.0).unwrap();
        assert_eq!(fs::read_to_string(&leftover).unwrap(), text);
        assert_eq!(fs::read_to_string(&ledger_leftover).unwrap(), ledger_text);
    }

    #[test]
    fn a_stale_temp_file_does_not_block_a_rewrite() {
        let scratch = scratch_dir("stale");
        let journal = journal_with_new_rows(&scratch.0);
        fs::write(temp_path(&journal), "junk").unwrap();
        rewrite_file(&journal, LogShape::Journal).unwrap();
        assert!(fs::read_to_string(&journal)
            .unwrap()
            .starts_with(old::JOURNAL_HEADER));
        assert!(!temp_path(&journal).exists());
    }
}
