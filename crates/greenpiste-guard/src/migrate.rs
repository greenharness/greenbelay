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
    fn name(&self) -> &str;
    /// Do the step; the `Ok` text is a note ("" when there is none).
    fn forward(&self, env: &Env) -> Result<String, String>;
    fn reverse(&self, env: &Env) -> Result<String, String>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    AlreadyDone,
    /// Each step is its name, with its note after a colon when it had one.
    Migrated {
        steps: Vec<String>,
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
    Locked { path: PathBuf },
    Io { what: String, source: io::Error },
    Step { step: String, message: String },
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
            MigrateError::Step { step, message } => {
                write!(f, "migration step {step} failed: {message}")
            }
        }
    }
}

impl std::error::Error for MigrateError {}

fn io_error(what: impl Into<String>) -> impl FnOnce(io::Error) -> MigrateError {
    let what = what.into();
    move |source| MigrateError::Io { what, source }
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
    ]
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
                    Ok(note) if note.is_empty() => reports.push(step.name().to_string()),
                    Ok(note) => reports.push(format!("{}: {note}", step.name())),
                    Err(message) => {
                        write_marker(&failed, &format!("{}\n{message}\n", step.name()))?;
                        return Err(MigrateError::Step {
                            step: step.name().to_string(),
                            message,
                        });
                    }
                }
            }
            write_marker(&done, "")?;
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
                step.reverse(env).map_err(|message| MigrateError::Step {
                    step: step.name().to_string(),
                    message,
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

fn fix_it(what: &str, source: io::Error) -> String {
    format!("{what}: {source}; fix it and run again")
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

    fn move_forward(&self, old_path: &Path, new_path: &Path) -> Result<String, String> {
        let old_is_dir = old_path
            .symlink_metadata()
            .ok()
            .is_some_and(|meta| meta.is_dir());
        let new_exists = exists(new_path);
        if old_is_dir && !new_exists {
            fs::rename(old_path, new_path).map_err(|e| {
                fix_it(
                    &format!(
                        "cannot rename {} to {}",
                        old_path.display(),
                        new_path.display()
                    ),
                    e,
                )
            })?;
            (self.after_rename)(old_path);
        } else if !exists(old_path) && !new_exists {
            return Ok(String::new());
        }
        match old_path.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => Ok(String::new()),
            Ok(meta) if meta.is_dir() => {
                self.absorb(old_path, new_path)?;
                link(old_path, new_path).map_err(|e| fix_it("cannot link", e))
            }
            Ok(_) => Err(format!(
                "{} is a file, not a directory; move it away and run again",
                old_path.display()
            )),
            Err(_) => link(old_path, new_path).map_err(|e| fix_it("cannot link", e)),
        }
    }

    /// Merge the directory `from` into `into`, then remove it.
    fn absorb(&self, from: &Path, into: &Path) -> Result<(), String> {
        self.merge(from, into)
            .map_err(|e| fix_it(&format!("cannot merge {}", from.display()), e))?;
        fs::remove_dir(from).map_err(|e| fix_it(&format!("cannot remove {}", from.display()), e))
    }

    /// Move every entry of `from` into `into`; an entry that exists in both
    /// stays as the one in `into`, and the other lands beside it.
    fn merge(&self, from: &Path, into: &Path) -> io::Result<()> {
        let seconds = (self.clock_seconds)();
        for entry in fs::read_dir(from)? {
            let name = entry?.file_name();
            let mut target = into.join(&name);
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
            fs::rename(from.join(&name), &target)?;
        }
        Ok(())
    }

    fn move_back(&self, old_path: &Path, new_path: &Path) -> Result<(), String> {
        match old_path.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => fs::remove_file(old_path)
                .map_err(|e| fix_it(&format!("cannot remove {}", old_path.display()), e))?,
            Ok(meta) if meta.is_dir() && exists(new_path) => self.absorb(old_path, new_path)?,
            _ => {}
        }
        if exists(new_path) && !exists(old_path) {
            fs::rename(new_path, old_path).map_err(|e| {
                fix_it(
                    &format!(
                        "cannot rename {} to {}",
                        new_path.display(),
                        old_path.display()
                    ),
                    e,
                )
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
    fn name(&self) -> &str {
        "state-directories"
    }

    fn forward(&self, env: &Env) -> Result<String, String> {
        let mut notes = Vec::new();
        for (old_path, new_path) in StateDirectories::roots(env) {
            let note = self.move_forward(&old_path, &new_path)?;
            if !note.is_empty() {
                notes.push(note);
            }
        }
        Ok(notes.join("; "))
    }

    fn reverse(&self, env: &Env) -> Result<String, String> {
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
                fix_it(
                    &format!("cannot rewrite the rows under {}", config_old.display()),
                    e,
                )
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
        let is_journal = entry
            .file_name()
            .to_string_lossy()
            .starts_with("journal.log");
        if is_journal && entry.file_type()?.is_file() {
            rewrite_file(&entry.path(), true)?;
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
        } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "jsonl") {
            rewrite_file(&entry.path(), false)?;
        }
    }
    Ok(())
}

/// Rewrite the rows of one file through a temp file renamed over it with the
/// original mode; a file with nothing to change is not touched.
fn rewrite_file(path: &Path, has_header: bool) -> io::Result<()> {
    let bytes = fs::read(path)?;
    let mut out = Vec::with_capacity(bytes.len());
    let mut changed = false;
    for (index, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let body_len = line.strip_suffix(b"\n").map_or(line.len(), <[u8]>::len);
        let rewritten = std::str::from_utf8(&line[..body_len])
            .ok()
            .and_then(|text| {
                if has_header && index == 0 && text == names::JOURNAL_HEADER {
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
    let mut temp_name = path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(".migrate-tmp");
    let temp = path.with_file_name(temp_name);
    fs::write(&temp, &out)?;
    fs::set_permissions(&temp, fs::metadata(path)?.permissions())?;
    fs::rename(&temp, path)
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
        fn walk(base: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let key = path.strip_prefix(base).unwrap().display().to_string();
                if path.is_dir() {
                    into.insert(format!("{key}/"), Vec::new());
                    walk(base, &path, into);
                } else {
                    into.insert(key, fs::read(&path).unwrap());
                }
            }
        }
        let mut found = BTreeMap::new();
        walk(dir, dir, &mut found);
        found
    }

    fn mtimes(dir: &Path) -> BTreeMap<PathBuf, SystemTime> {
        fn walk(dir: &Path, into: &mut BTreeMap<PathBuf, SystemTime>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                into.insert(path.clone(), modified);
                if path.is_dir() {
                    walk(&path, into);
                }
            }
        }
        let mut found = BTreeMap::new();
        walk(dir, &mut found);
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
        assert_eq!(
            steps,
            vec![
                "state-directories".to_string(),
                "git config keys".to_string()
            ]
        );
        assert!(elapsed < Duration::from_secs(60));
        assert_eq!(snapshot(&config_new(&env)), before);
        assert_eq!(snapshot(&env.state_root), state_before);
        assert_eq!(fs::read_link(config_old(&env)).unwrap(), config_new(&env));
        assert_eq!(fs::read_link(state_old(&env)).unwrap(), env.state_root);
        assert!(exists(&env.config_dir.join(names::MIGRATE_DONE_MARKER)));
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
        fn name(&self) -> &str {
            "flaky"
        }
        fn forward(&self, _env: &Env) -> Result<String, String> {
            self.attempts.set(self.attempts.get() + 1);
            if self.attempts.get() == 1 {
                Err("disk says no".to_string())
            } else {
                Ok(String::new())
            }
        }
        fn reverse(&self, _env: &Env) -> Result<String, String> {
            Ok(String::new())
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
}
