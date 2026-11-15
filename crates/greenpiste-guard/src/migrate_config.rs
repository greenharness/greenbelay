//! The git-config step of the one-time migration (ADR-0068 `harness.renames`):
//! every old-prefix key moves to the new prefix in the very file that holds
//! it, includes and conditions kept. Per key it copies, verifies the copy by
//! reading it back, and only then removes the old key, so an interruption
//! leaves the old keys intact and a re-run finishes without a duplicate.
//!
//! Git does the reading and the writing (`git config --file`); this module
//! only decides which files and which keys. Every git call is built from
//! [`Env`], so a test runs against scratch files and never the real config.

use crate::migrate::{Direction, Env, Step};
use crate::names::{self, old};
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Git's own limit on nested includes.
const MAX_INCLUDE_DEPTH: usize = 10;

/// Called with the file between a verified copy and the removal of the old key.
type Between = fn(&Path) -> Result<(), String>;

fn no_hook(_file: &Path) -> Result<(), String> {
    Ok(())
}

/// The step: rename this machine's old git-config keys in their own files.
pub struct ConfigKeys {
    between: Between,
}

impl ConfigKeys {
    pub fn new() -> ConfigKeys {
        ConfigKeys { between: no_hook }
    }

    /// A step that calls `hook` between the verified copy and the removal of
    /// each old key, for a test that interrupts there.
    pub fn with_hook_between_copy_and_unset(hook: Between) -> ConfigKeys {
        ConfigKeys { between: hook }
    }
}

impl Default for ConfigKeys {
    fn default() -> Self {
        ConfigKeys::new()
    }
}

impl Step for ConfigKeys {
    fn name(&self) -> &str {
        "git config keys"
    }

    fn forward(&self, env: &Env) -> Result<String, String> {
        migrate_machine(env, Rename::FORWARD, self.between)
    }

    fn reverse(&self, env: &Env) -> Result<String, String> {
        migrate_machine(env, Rename::REVERSE, self.between)
    }

    fn plan(&self, env: &Env, direction: Direction) -> Result<Vec<String>, String> {
        plan_machine(env, Rename::for_direction(direction))
    }
}

/// Rename the keys in a repository's own config and worktree config. For a
/// `greenpiste migrate` run inside a repository; not part of `steps()`.
pub fn migrate_local(env: &Env, repo: &Path, direction: Direction) -> Result<String, String> {
    let rename = Rename::for_direction(direction);
    let mut errors = Vec::new();
    let mut moved = 0;
    for name in ["config", "config.worktree"] {
        let located = git_command(env)
            .current_dir(repo)
            .args(["rev-parse", "--git-path", name])
            .output()
            .map_err(|e| format!("cannot run git: {e}"))?;
        if !located.status.success() {
            return Err(format!(
                "{} is not a git repository: {}",
                repo.display(),
                stderr_text(&located)
            ));
        }
        let text = String::from_utf8_lossy(&located.stdout).trim().to_string();
        let path = repo.join(text);
        if !path.is_file() {
            continue;
        }
        match migrate_file(env, &path, rename, no_hook) {
            Ok(count) => moved += count,
            Err(message) => errors.push(message),
        }
    }
    finish(moved, errors)
}

#[derive(Clone, Copy)]
struct Rename {
    from: &'static str,
    to: &'static str,
}

impl Rename {
    const FORWARD: Rename = Rename {
        from: old::CONFIG_PREFIX,
        to: names::CONFIG_PREFIX,
    };
    const REVERSE: Rename = Rename {
        from: names::CONFIG_PREFIX,
        to: old::CONFIG_PREFIX,
    };

    fn for_direction(direction: Direction) -> Rename {
        match direction {
            Direction::Forward => Rename::FORWARD,
            Direction::Reverse => Rename::REVERSE,
        }
    }

    /// The key's new name when it carries the old prefix: the section is
    /// matched without case (git lists it lowercased), the rest as spelled.
    fn apply(&self, key: &str) -> Option<String> {
        let (from_section, from_rest) = self.from.split_once('.')?;
        let (section, rest) = key.split_once('.')?;
        if section.eq_ignore_ascii_case(from_section)
            && rest.starts_with(from_rest)
            && rest.len() > from_rest.len()
        {
            Some(format!("{}{}", self.to, &rest[from_rest.len()..]))
        } else {
            None
        }
    }
}

struct Entry {
    origin: String,
    key: String,
    value: Option<String>,
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

/// A `git` command under the Env's explicit environment: with a list, the
/// process environment is cleared and only that list plus `PATH` remains.
fn git_command(env: &Env) -> Command {
    let mut command = Command::new("git");
    if let Some(list) = &env.git_env {
        command.env_clear();
        command.envs(list.iter().map(|(name, value)| (name, value)));
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
    }
    command
}

/// One variable as git sees it under the Env.
fn env_var(env: &Env, name: &str) -> Option<OsString> {
    match &env.git_env {
        Some(list) => list
            .iter()
            .find(|(key, _)| key.as_os_str() == name)
            .map(|(_, value)| value.clone()),
        None => std::env::var_os(name),
    }
}

/// The entries of a `git config --list -z` output.
fn parse_entries(stdout: &[u8], with_origin: bool) -> Vec<Entry> {
    let mut tokens: Vec<&[u8]> = stdout.split(|byte| *byte == 0).collect();
    if tokens.last().is_some_and(|last| last.is_empty()) {
        tokens.pop();
    }
    let width = if with_origin { 2 } else { 1 };
    tokens
        .chunks(width)
        .filter(|chunk| chunk.len() == width)
        .map(|chunk| {
            let (origin, pair) = if with_origin {
                (String::from_utf8_lossy(chunk[0]).into_owned(), chunk[1])
            } else {
                (String::new(), chunk[0])
            };
            let text = String::from_utf8_lossy(pair).into_owned();
            match text.split_once('\n') {
                Some((key, value)) => Entry {
                    origin,
                    key: key.to_string(),
                    value: Some(value.to_string()),
                },
                None => Entry {
                    origin,
                    key: text,
                    value: None,
                },
            }
        })
        .collect()
}

fn run_git(mut command: Command, what: &str) -> Result<Output, String> {
    command
        .output()
        .map_err(|e| format!("cannot run git to {what}: {e}"))
}

/// A list that fails with no message, or on a file that is not there, is an empty list, not a failure.
fn list_output(output: Output, what: &str) -> Result<Vec<u8>, String> {
    let absent = stderr_text(&output).contains("No such file or directory");
    if output.status.success() || stderr_text(&output).is_empty() || absent {
        Ok(output.stdout)
    } else {
        Err(format!("git could not {what}: {}", stderr_text(&output)))
    }
}

/// The top-level files of the global and system scopes, and the keys that
/// come from the command scope (which no file holds).
fn inventory(env: &Env, rename: Rename) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    let mut scopes = vec!["--global"];
    if env_var(env, "GIT_CONFIG_NOSYSTEM").is_none() {
        scopes.push("--system");
    }
    let mut tops: Vec<PathBuf> = Vec::new();
    for scope in scopes {
        let mut command = git_command(env);
        command.args([
            "config",
            "--list",
            scope,
            "--show-origin",
            "--no-includes",
            "-z",
        ]);
        let stdout = list_output(run_git(command, "list its config")?, "list its config")?;
        for entry in parse_entries(&stdout, true) {
            if let Some(file) = entry.origin.strip_prefix("file:") {
                let file = PathBuf::from(file);
                if !tops.contains(&file) {
                    tops.push(file);
                }
            }
        }
    }
    let mut command = git_command(env);
    command.current_dir(std::env::temp_dir()).args([
        "config",
        "--list",
        "--show-origin",
        "--no-includes",
        "-z",
    ]);
    let stdout = list_output(run_git(command, "list its config")?, "list its config")?;
    let mut command_keys = Vec::new();
    for entry in parse_entries(&stdout, true) {
        if entry.origin.starts_with("command") && rename.apply(&entry.key).is_some() {
            command_keys.push(entry.key);
        }
    }
    Ok((tops, command_keys))
}

fn file_command(env: &Env, path: &Path) -> Command {
    let mut command = git_command(env);
    command.arg("config").arg("--file").arg(path);
    command
}

fn list_file(env: &Env, path: &Path) -> Result<Vec<Entry>, String> {
    let mut command = file_command(env, path);
    command.args(["--list", "--no-includes", "-z"]);
    let output = run_git(command, "read a config file")?;
    if !output.status.success() {
        return Err(format!(
            "git could not read {}: {}",
            path.display(),
            stderr_text(&output)
        ));
    }
    Ok(parse_entries(&output.stdout, false))
}

fn is_include_key(key: &str) -> bool {
    key == "include.path" || (key.starts_with("includeif.") && key.ends_with(".path"))
}

fn resolve_include(env: &Env, including: &Path, value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = env_var(env, "HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    let target = PathBuf::from(value);
    if target.is_absolute() {
        target
    } else {
        including
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(target)
    }
}

fn visit(
    env: &Env,
    path: PathBuf,
    depth: usize,
    seen: &mut HashSet<PathBuf>,
    order: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if depth > MAX_INCLUDE_DEPTH {
        return Ok(());
    }
    let Ok(real) = fs::canonicalize(&path) else {
        return Ok(()); // an include of a file that does not exist is skipped, as git does
    };
    if !real.is_file() || !seen.insert(real) {
        return Ok(());
    }
    let entries = list_file(env, &path)?;
    order.push(path.clone());
    for entry in &entries {
        if let (true, Some(value)) = (is_include_key(&entry.key), &entry.value) {
            visit(
                env,
                resolve_include(env, &path, value),
                depth + 1,
                seen,
                order,
            )?;
        }
    }
    Ok(())
}

/// Every file the top-level files include, whether or not the condition of
/// an `includeIf` holds here.
fn collect_files(env: &Env, tops: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    let mut seen = HashSet::new();
    let mut order = Vec::new();
    for top in tops {
        visit(env, top, 0, &mut seen, &mut order)?;
    }
    Ok(order)
}

fn migrate_machine(env: &Env, rename: Rename, between: Between) -> Result<String, String> {
    let (tops, command_keys) = inventory(env, rename)?;
    let files = collect_files(env, tops)?;
    let mut errors = Vec::new();
    let mut moved = 0;
    for file in &files {
        match migrate_file(env, file, rename, between) {
            Ok(count) => moved += count,
            Err(message) => errors.push(message),
        }
    }
    for key in command_keys {
        errors.push(format!(
            "{key} is set by the command line or GIT_CONFIG_COUNT, which no file holds; \
             remove that setting (the -c option or the GIT_CONFIG_KEY_n variable) and run again"
        ));
    }
    finish(moved, errors)
}

/// What [`migrate_machine`] would rename, read through `git config --list`
/// only; no file is opened for writing.
fn plan_machine(env: &Env, rename: Rename) -> Result<Vec<String>, String> {
    let (tops, command_keys) = inventory(env, rename)?;
    let mut lines = Vec::new();
    for file in collect_files(env, tops)? {
        let mut seen: Vec<String> = Vec::new();
        for entry in list_file(env, &file)? {
            let Some(new_key) = rename.apply(&entry.key) else {
                continue;
            };
            if !seen.contains(&entry.key) {
                lines.push(format!(
                    "rename {} to {new_key} in {}",
                    entry.key,
                    file.display()
                ));
                seen.push(entry.key);
            }
        }
    }
    for key in command_keys {
        lines.push(format!(
            "refuse: {key} is set by the command line or GIT_CONFIG_COUNT, which no file holds"
        ));
    }
    Ok(lines)
}

/// The old-name keys this machine's config files still hold, each with the
/// file that holds it, found the way the migration step finds them. Read-only;
/// anything git cannot answer is reported as no keys.
pub fn old_keys(env: &Env) -> Vec<(String, PathBuf)> {
    let rename = Rename::FORWARD;
    let Ok((tops, _)) = inventory(env, rename) else {
        return Vec::new();
    };
    let Ok(files) = collect_files(env, tops) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for file in files {
        let Ok(entries) = list_file(env, &file) else {
            continue;
        };
        for entry in entries {
            if rename.apply(&entry.key).is_some() {
                let hit = (entry.key, file.clone());
                if !found.contains(&hit) {
                    found.push(hit);
                }
            }
        }
    }
    found
}

fn finish(moved: usize, errors: Vec<String>) -> Result<String, String> {
    if errors.is_empty() {
        Ok(if moved == 0 {
            String::new()
        } else {
            format!("renamed {moved} git config key(s)")
        })
    } else {
        Err(errors.join("; "))
    }
}

fn get_all(env: &Env, path: &Path, key: &str) -> Result<Vec<String>, String> {
    let mut command = file_command(env, path);
    command.args(["--get-all", "-z", key]);
    let output = run_git(command, "read a config key")?;
    match output.status.code() {
        Some(0) => {
            let mut values: Vec<String> = output
                .stdout
                .split(|byte| *byte == 0)
                .map(|value| String::from_utf8_lossy(value).into_owned())
                .collect();
            values.pop();
            Ok(values)
        }
        Some(1) => Ok(Vec::new()),
        _ => Err(format!(
            "git could not read {key} in {}: {}",
            path.display(),
            stderr_text(&output)
        )),
    }
}

fn write_step(env: &Env, path: &Path, args: &[&str]) -> Result<(), String> {
    let mut command = file_command(env, path);
    command.args(args);
    let output = run_git(command, "write a config key")?;
    // `--unset-all` of an absent key exits 5: the removal is already done.
    if output.status.success() || output.status.code() == Some(5) {
        Ok(())
    } else {
        Err(format!(
            "git could not write {}: {}",
            path.display(),
            stderr_text(&output)
        ))
    }
}

fn ensure_writable(path: &Path, key_count: usize) -> Result<(), String> {
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    fs::OpenOptions::new()
        .append(true)
        .open(&real)
        .map(drop)
        .map_err(|e| {
            format!(
                "{} holds {key_count} key(s) to rename but is not writable ({e}); make it \
                 writable for this user (or run the migration as its owner) and run again",
                path.display()
            )
        })
}

/// Rename the keys of one file; returns how many keys it renamed. A file
/// with none is never opened for writing.
fn migrate_file(env: &Env, path: &Path, rename: Rename, between: Between) -> Result<usize, String> {
    let mut keys: Vec<(String, String)> = Vec::new();
    for entry in list_file(env, path)? {
        let Some(new_key) = rename.apply(&entry.key) else {
            continue;
        };
        if entry.value.is_none() {
            return Err(format!(
                "{} in {} has no value, which a copy cannot keep; give it one by hand and run again",
                entry.key,
                path.display()
            ));
        }
        if !keys.iter().any(|(old_key, _)| *old_key == entry.key) {
            keys.push((entry.key, new_key));
        }
    }
    if keys.is_empty() {
        return Ok(0);
    }
    ensure_writable(path, keys.len())?;
    for (old_key, new_key) in &keys {
        let values = get_all(env, path, old_key)?;
        let mut present = get_all(env, path, new_key)?;
        for value in &values {
            if !present.contains(value) {
                write_step(env, path, &["--add", new_key, value])?;
                present.push(value.clone());
            }
        }
        let copied = get_all(env, path, new_key)?;
        if let Some(missing) = values.iter().find(|value| !copied.contains(value)) {
            return Err(format!(
                "{new_key} in {} does not hold {missing:?} after the copy; {old_key} is left in place",
                path.display()
            ));
        }
        between(path)?;
        write_step(env, path, &["--unset-all", old_key])?;
    }
    Ok(keys.len())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::migrate::{machine_in, Outcome, Wait};
    use std::collections::BTreeMap;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::SystemTime;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct Scratch(PathBuf, std::sync::MutexGuard<'static, ()>);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _serial = &self.1; // the guard outlives the cleanup
            let _ = fs::remove_dir_all(&self.0); // best-effort cleanup of a temp dir
        }
    }

    fn pairs(list: &[(&str, &Path)]) -> Vec<(OsString, OsString)> {
        list.iter()
            .map(|(name, path)| (OsString::from(name), path.as_os_str().to_os_string()))
            .collect()
    }

    /// A scratch machine: HOME and the config roots inside one temp dir, the
    /// system config switched off.
    fn machine() -> (Scratch, PathBuf, Env) {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("gp-config-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&root); // a leftover of a crashed run
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(root.join("xdg")).unwrap();
        let mut git_env = pairs(&[
            ("HOME", &home),
            ("XDG_CONFIG_HOME", &root.join("xdg")),
            ("GIT_CONFIG_GLOBAL", &home.join(".gitconfig")),
            ("GIT_CONFIG_SYSTEM", &root.join("system.gitconfig")),
        ]);
        git_env.push(("GIT_CONFIG_NOSYSTEM".into(), "1".into()));
        let env = Env {
            config_dir: root.join("claude"),
            state_root: root.join("state").join(names::DATA_DIR),
            git_env: Some(git_env),
        };
        (Scratch(root.clone(), crate::migrate::serial()), root, env)
    }

    fn old_key(rest: &str) -> String {
        format!("{}{rest}", old::CONFIG_PREFIX)
    }

    fn new_key(rest: &str) -> String {
        format!("{}{rest}", names::CONFIG_PREFIX)
    }

    fn git_ok(env: &Env, cwd: &Path, args: &[&str]) -> String {
        let output = git_command(env)
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            stderr_text(&output)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// The effective values of every key, grouped by key, in listed order.
    fn effective(env: &Env, cwd: &Path) -> BTreeMap<String, Vec<String>> {
        let listing = git_ok(env, cwd, &["config", "--list", "--global", "--includes"]);
        let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for line in listing.lines() {
            let (key, value) = line.split_once('=').unwrap_or((line, ""));
            found
                .entry(key.to_string())
                .or_default()
                .push(value.to_string());
        }
        found
    }

    fn rename_map(
        before: &BTreeMap<String, Vec<String>>,
        rename: Rename,
    ) -> BTreeMap<String, Vec<String>> {
        before
            .iter()
            .map(|(key, values)| (rename.apply(key).unwrap_or(key.clone()), values.clone()))
            .collect()
    }

    struct Fixture {
        repo: PathBuf,
        global: PathBuf,
        active: PathBuf,
        inactive: PathBuf,
    }

    fn fixture(root: &Path, env: &Env) -> Fixture {
        let home = root.join("home");
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git_ok(env, &repo, &["init", "-q"]);
        let global = home.join(".gitconfig");
        let active = home.join("active.gitconfig");
        let inactive = home.join("inactive.gitconfig");
        fs::write(
            &global,
            format!(
                "# my config\n[user]\n\tname = Test User\n[amont \"agent\"]\n\tenabled = true\n\
                 \topener = open\n[amont \"agent.git-add-broad\"]\n\tstance = deny\n\
                 [amont \"agent.symbol-grep-not-lsp\"]\n\tdenyAgents = haiku\n\tdenyAgents = sonnet\n\
                 [core]\n\teditor = vi\n[includeIf \"gitdir:{}/\"]\n\tpath = active.gitconfig\n\
                 [includeIf \"gitdir:/nowhere/at/all/\"]\n\tpath = ~/inactive.gitconfig\n",
                repo.display()
            ),
        )
        .unwrap();
        fs::write(&active, "[amont \"agent\"]\n\tactivekey = 1\n# keep me\n").unwrap();
        fs::write(
            &inactive,
            "[alias]\n\tst = status\n[amont \"agent\"]\n\tinactivekey = 2\n",
        )
        .unwrap();
        Fixture {
            repo,
            global,
            active,
            inactive,
        }
    }

    fn has_keys(env: &Env, file: &Path, rename: Rename) -> bool {
        list_file(env, file)
            .unwrap()
            .iter()
            .any(|entry| rename.apply(&entry.key).is_some())
    }

    fn holds(path: &Path, needle: &str) -> bool {
        fs::read_to_string(path).unwrap().contains(needle)
    }

    fn file_times(files: &[&Path]) -> Vec<(Vec<u8>, SystemTime)> {
        files
            .iter()
            .map(|file| {
                (
                    fs::read(file).unwrap(),
                    fs::metadata(file).unwrap().modified().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn forward_renames_every_file_keeping_effective_values() {
        let (_scratch, root, env) = machine();
        let fx = fixture(&root, &env);
        let before = effective(&env, &fx.repo);
        assert!(before.contains_key(&old_key("enabled")));
        assert_eq!(before[&old_key("symbol-grep-not-lsp.denyagents")].len(), 2);
        assert!(before.contains_key(&old_key("activekey")));
        let unrelated = fs::read_to_string(&fx.global).unwrap();

        let note = ConfigKeys::new().forward(&env).unwrap();
        assert!(note.contains("renamed"), "{note}");

        let after = effective(&env, &fx.repo);
        assert_eq!(after, rename_map(&before, Rename::FORWARD));
        assert!(after.contains_key(&new_key("git-add-broad.stance")));
        for file in [&fx.global, &fx.active, &fx.inactive] {
            assert!(!has_keys(&env, file, Rename::FORWARD), "{}", file.display());
        }
        assert!(holds(&fx.inactive, "st = status"));
        assert!(holds(&fx.active, "# keep me"));
        let kept: Vec<&str> = unrelated
            .lines()
            .filter(|line| {
                !line.contains("amont") && !line.contains("enabled") && {
                    let trimmed = line.trim();
                    !["opener", "stance", "denyAgents"]
                        .iter()
                        .any(|name| trimmed.starts_with(name))
                }
            })
            .collect();
        let after_text = fs::read_to_string(&fx.global).unwrap();
        let mut remaining = after_text.lines();
        for line in kept {
            assert!(
                remaining.any(|candidate| candidate == line),
                "lost {line:?}"
            );
        }
    }

    #[test]
    fn a_second_forward_writes_no_byte() {
        let (_scratch, root, env) = machine();
        let fx = fixture(&root, &env);
        ConfigKeys::new().forward(&env).unwrap();
        let files = [
            fx.global.as_path(),
            fx.active.as_path(),
            fx.inactive.as_path(),
        ];
        let snapshot = file_times(&files);
        assert_eq!(ConfigKeys::new().forward(&env).unwrap(), "");
        assert_eq!(file_times(&files), snapshot);
    }

    #[test]
    fn reverse_restores_the_old_keys_and_leaves_no_new_one() {
        let (_scratch, root, env) = machine();
        let fx = fixture(&root, &env);
        let before = effective(&env, &fx.repo);
        ConfigKeys::new().forward(&env).unwrap();
        ConfigKeys::new().reverse(&env).unwrap();
        assert_eq!(effective(&env, &fx.repo), before);
        for file in [&fx.global, &fx.active, &fx.inactive] {
            assert!(!has_keys(&env, file, Rename::REVERSE), "{}", file.display());
        }
    }

    fn interrupt(_file: &Path) -> Result<(), String> {
        Err("interrupted".to_string())
    }

    #[test]
    fn an_interrupted_copy_leaves_old_keys_and_a_rerun_finishes_without_duplicates() {
        let (_scratch, root, env) = machine();
        let fx = fixture(&root, &env);
        let before = effective(&env, &fx.repo);
        let error = ConfigKeys::with_hook_between_copy_and_unset(interrupt)
            .forward(&env)
            .unwrap_err();
        assert!(error.contains("interrupted"));
        let midway = effective(&env, &fx.repo);
        assert!(midway.contains_key(&old_key("enabled")));
        ConfigKeys::new().forward(&env).unwrap();
        let after = effective(&env, &fx.repo);
        assert_eq!(after, rename_map(&before, Rename::FORWARD));
        assert_eq!(after[&new_key("symbol-grep-not-lsp.denyagents")].len(), 2);
    }

    #[test]
    fn a_symlinked_file_stays_a_symlink_and_its_target_changes() {
        let (_scratch, root, mut env) = machine();
        let target = root.join("dotfiles-gitconfig");
        fs::write(&target, "[amont \"agent\"]\n\tenabled = true\n").unwrap();
        let link = root.join("home").join(".gitconfig");
        symlink(&target, &link).unwrap();
        if let Some(list) = env.git_env.as_mut() {
            list.retain(|(name, _)| name != "GIT_CONFIG_GLOBAL");
            list.push(("GIT_CONFIG_GLOBAL".into(), link.as_os_str().to_os_string()));
        }
        ConfigKeys::new().forward(&env).unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(holds(&target, "greenpiste"));
        assert!(!holds(&target, "amont"));
    }

    #[test]
    fn an_unwritable_system_file_fails_after_the_rest_is_migrated() {
        let (_scratch, root, mut env) = machine();
        let fx = fixture(&root, &env);
        let system = root.join("system.gitconfig");
        fs::write(&system, "[amont \"agent\"]\n\tsystemkey = 3\n").unwrap();
        fs::set_permissions(&system, fs::Permissions::from_mode(0o444)).unwrap();
        if let Some(list) = env.git_env.as_mut() {
            list.retain(|(name, _)| name != "GIT_CONFIG_NOSYSTEM");
        }
        let error = ConfigKeys::new().forward(&env).unwrap_err();
        assert!(error.contains("system.gitconfig") && error.contains("not writable"));
        assert!(holds(&system, "amont"));
        assert!(!holds(&fx.global, "amont"));
    }

    #[test]
    fn a_read_only_file_with_no_old_key_is_never_written() {
        let (_scratch, root, env) = machine();
        let fx = fixture(&root, &env);
        let plain = root.join("home").join("plain.gitconfig");
        fs::write(&plain, "[core]\n\teditor = vi\n").unwrap();
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o444)).unwrap();
        let mut global = fs::read_to_string(&fx.global).unwrap();
        global.push_str("[include]\n\tpath = plain.gitconfig\n");
        fs::write(&fx.global, global).unwrap();
        ConfigKeys::new().forward(&env).unwrap();
    }

    #[test]
    fn a_command_scope_key_fails_naming_the_source() {
        let (_scratch, root, mut env) = machine();
        let fx = fixture(&root, &env);
        if let Some(list) = env.git_env.as_mut() {
            list.push(("GIT_CONFIG_COUNT".into(), "1".into()));
            list.push(("GIT_CONFIG_KEY_0".into(), old_key("fromenv").into()));
            list.push(("GIT_CONFIG_VALUE_0".into(), "1".into()));
        }
        let error = ConfigKeys::new().forward(&env).unwrap_err();
        assert!(error.contains(&old_key("fromenv")) && error.contains("GIT_CONFIG_COUNT"));
        assert!(!holds(&fx.global, "amont"));
    }

    #[test]
    fn the_real_users_gitconfig_is_never_read() {
        let (_scratch, root, mut env) = machine();
        if let Some(list) = env.git_env.as_mut() {
            list.retain(|(name, _)| name != "GIT_CONFIG_GLOBAL");
        }
        let scratch_global = root.join("home").join(".gitconfig");
        fs::write(&scratch_global, "[amont \"agent\"]\n\tenabled = true\n").unwrap();
        ConfigKeys::new().forward(&env).unwrap();
        assert!(holds(&scratch_global, "greenpiste"));
        assert!(!holds(&scratch_global, "amont"));
    }

    #[test]
    fn local_migration_renames_the_repositorys_own_config() {
        let (_scratch, root, env) = machine();
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git_ok(&env, &repo, &["init", "-q"]);
        git_ok(
            &env,
            &repo,
            &["config", "--local", &old_key("enabled"), "true"],
        );
        migrate_local(&env, &repo, Direction::Forward).unwrap();
        assert_eq!(
            git_ok(&env, &repo, &["config", "--local", &new_key("enabled")]).trim(),
            "true"
        );
        assert!(!holds(&repo.join(".git/config"), "amont"));
        migrate_local(&env, &repo, Direction::Reverse).unwrap();
        assert!(holds(&repo.join(".git/config"), "amont"));
        assert!(!holds(&repo.join(".git/config"), "greenpiste"));
    }

    #[test]
    fn the_runner_runs_the_state_step_first_then_the_config_step() {
        let (_scratch, root, env) = machine();
        let fx = fixture(&root, &env);
        let Outcome::Migrated { steps, .. } =
            machine_in(&env, Direction::Forward, Wait::Zero).unwrap()
        else {
            panic!("not migrated");
        };
        assert_eq!(steps[0], "state-directories");
        assert!(steps[1].starts_with("git config keys"));
        assert!(!holds(&fx.global, "amont"));
    }
}
