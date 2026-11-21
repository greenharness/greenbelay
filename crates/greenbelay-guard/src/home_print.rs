//! A fingerprint of the few home files a test run must never change, taken
//! before a command and again after it, for `home-fingerprint-drift`.
//!
//! Two incidents made this: a leaky test run migrated the real home
//! (2026-11-15), and a migrating binary's tests touched the live repository
//! (2026-11-11). Neither failed anything.
//! relais takes the same snapshot around a run (its `homeprint.rs`); no code is
//! shared, so each tool works alone, and both document the same watched set.
//!
//! ## What is watched
//!
//! `~/.gitconfig`, `$XDG_CONFIG_HOME/git/config` (else `~/.config/git/config`),
//! `~/.netrc` (and `~/_netrc` on Windows), `~/.claude/settings*.json`,
//! `~/.claude/plugins/{installed_plugins,known_marketplaces}.json`, every file
//! under `~/.ssh` but `known_hosts*` (ssh appends to it), and the repository's
//! shared `.git/config`. `~` is `HOME`, else `USERPROFILE`; `~/.claude` is
//! `CLAUDE_CONFIG_DIR` when set. `~/.config` as a whole is not watched: caches
//! rewrite it all day.
//!
//! ## What a snapshot holds
//!
//! Hashes only, never contents: sha256 and size of each regular file, its mode
//! bits on unix, a symlink by the hash of its target (never followed), an
//! absent path as absent. A file over [`HASH_LIMIT`] is fingerprinted by its
//! size and modification time instead, so a stray large file cannot cost the
//! hook its budget. Only regular files are opened: a FIFO would block the read
//! and an ssh ControlMaster socket comes and goes, so both are left out. A
//! directory expands to at most [`CAP`] files in byte order; past that it is
//! marked truncated.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// The most files one watched directory expands to.
pub const CAP: usize = 500;

/// The largest file read and hashed; a larger one is fingerprinted by size
/// and modification time.
pub const HASH_LIMIT: u64 = 1024 * 1024;

/// The first line of every record, so a record another version wrote is
/// never read as a snapshot with paths missing.
const HEADER: &str = "homeprint/1";

/// One watched place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Root {
    /// A single path.
    File(PathBuf),
    /// Every file under a directory, recursively, but those whose name
    /// starts with one of `skip`.
    Tree { dir: PathBuf, skip: Vec<String> },
    /// The files directly in `dir` whose name starts with `prefix` and ends
    /// with `suffix` (`~/.claude/settings*.json`).
    Glob {
        dir: PathBuf,
        prefix: String,
        suffix: String,
    },
}

/// What one path was when the snapshot was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// A regular file up to [`HASH_LIMIT`]: content hash, size, and mode
    /// bits (0 off unix).
    File { sha: String, size: u64, mode: u32 },
    /// A regular file over [`HASH_LIMIT`], never read: modification time in
    /// nanoseconds since the epoch, size, and mode bits.
    Large {
        modified_ns: u128,
        size: u64,
        mode: u32,
    },
    /// A symlink, by the hash of its target.
    Link(String),
    /// The path exists but could not be read.
    Unreadable,
}

/// A snapshot: each path's state, and the roots cut at [`CAP`]. An absent
/// path has no entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub roots: Vec<Root>,
    pub paths: BTreeMap<PathBuf, State>,
    pub truncated: Vec<PathBuf>,
}

/// What happened to one path between two snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Added,
    Removed,
    Modified,
    /// Readable on one side only.
    Unreadable,
    /// A watched directory over [`CAP`] files: past the cap is not checked.
    Truncated,
}

impl Change {
    pub fn as_str(self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Removed => "removed",
            Change::Modified => "modified",
            Change::Unreadable => "not checked: unreadable",
            Change::Truncated => "not checked: over 500 files",
        }
    }

    /// A change of the file itself, as opposed to a note that it could not
    /// be checked.
    pub fn is_real(self) -> bool {
        match self {
            Change::Added | Change::Removed | Change::Modified => true,
            Change::Unreadable | Change::Truncated => false,
        }
    }
}

/// An environment lookup: the real `std::env::var_os`, or a test's.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// The home directory: `HOME`, else `USERPROFILE`, an empty value counting
/// as unset.
pub fn home(var: Env<'_>) -> Option<PathBuf> {
    set(var, "HOME").or_else(|| set(var, "USERPROFILE"))
}

fn set(var: Env<'_>, key: &str) -> Option<PathBuf> {
    var(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The watched set for this environment. `var` reads an environment
/// variable (a closure, so tests never call `set_var`); `git_dir` is the
/// repository's shared git directory, when the command ran in one.
pub fn watched(var: Env<'_>, git_dir: Option<&Path>) -> Vec<Root> {
    let mut roots = Vec::new();
    if let Some(home) = home(var) {
        roots.push(Root::File(home.join(".gitconfig")));
        roots.push(Root::File(home.join(".netrc")));
        if cfg!(windows) {
            roots.push(Root::File(home.join("_netrc")));
        }
        roots.push(Root::Tree {
            dir: home.join(".ssh"),
            skip: vec!["known_hosts".to_string()],
        });
        let xdg = set(var, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        roots.push(Root::File(xdg.join("git").join("config")));
        let claude = set(var, "CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude"));
        roots.push(Root::Glob {
            dir: claude.clone(),
            prefix: "settings".to_string(),
            suffix: ".json".to_string(),
        });
        let plugins = claude.join("plugins");
        roots.push(Root::File(plugins.join("installed_plugins.json")));
        roots.push(Root::File(plugins.join("known_marketplaces.json")));
    } else if let Some(xdg) = set(var, "XDG_CONFIG_HOME") {
        roots.push(Root::File(xdg.join("git").join("config")));
    }
    if let Some(git_dir) = git_dir {
        roots.push(Root::File(git_dir.join("config")));
    }
    roots
}

/// The shared git directory of the repository holding `cwd`, absolute: a
/// linked worktree answers with its main repository's, whose `config` both
/// share. `None` outside a repository, and then no repository config is
/// watched.
///
/// Read from disk the way git finds it (the nearest `.git` up from `cwd`; a
/// `.git` file's `gitdir:` line; that directory's `commondir` file), not by
/// spawning `git rev-parse --git-common-dir`: the spawn measured 46 ms p50 on
/// a loaded host, more than the hook's whole 20 ms budget. `GIT_DIR` set for
/// the command alone is not seen; the hook only knows its cwd.
pub fn git_common_dir(cwd: &Path) -> Option<PathBuf> {
    let cwd = std::path::absolute(cwd).ok()?;
    let dot_git = cwd
        .ancestors()
        .map(|dir| dir.join(".git"))
        .find(|candidate| candidate.exists())?;
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let target = text
            .lines()
            .find_map(|line| line.strip_prefix("gitdir:"))?
            .trim();
        dot_git.parent()?.join(target)
    };
    let common = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => git_dir.join(text.trim_end_matches(['\n', '\r'])),
        Err(_) => git_dir,
    };
    if !common.is_dir() {
        return None;
    }
    // Windows: `canonicalize` answers a `\\?\` path, which prints badly;
    // `absolute` resolves `..` there without it.
    if cfg!(windows) {
        std::path::absolute(&common).ok()
    } else {
        std::fs::canonicalize(&common).ok()
    }
}

/// How the snapshot reads a file: the real `std::fs::read`, or a test's.
pub type Reader<'a> = &'a dyn Fn(&Path) -> io::Result<Vec<u8>>;

/// Take a snapshot of `roots` from the real filesystem.
pub fn snapshot(roots: &[Root]) -> Snapshot {
    snapshot_with(roots, &|path| std::fs::read(path))
}

/// [`snapshot`] through `read`, the seam a test injects a read error with.
pub fn snapshot_with(roots: &[Root], read: Reader<'_>) -> Snapshot {
    let mut snapshot = Snapshot {
        roots: roots.to_vec(),
        ..Snapshot::default()
    };
    for root in roots {
        match root {
            Root::File(path) => {
                if let Some(state) = state_of(path, read) {
                    snapshot.paths.insert(path.clone(), state);
                }
            }
            Root::Tree { dir, skip } => {
                let mut walker = Walker {
                    skip,
                    read,
                    found: BTreeMap::new(),
                };
                if !walker.walk(dir) {
                    snapshot.truncated.push(dir.clone());
                }
                snapshot.paths.extend(walker.found);
            }
            Root::Glob {
                dir,
                prefix,
                suffix,
            } => {
                for path in sorted_entries(dir) {
                    let fits = path.file_name().is_some_and(|name| {
                        let name = name.to_string_lossy();
                        name.starts_with(prefix.as_str()) && name.ends_with(suffix.as_str())
                    });
                    if fits {
                        if let Some(state) = state_of(&path, read) {
                            snapshot.paths.insert(path, state);
                        }
                    }
                }
            }
        }
    }
    snapshot
}

/// The entries of `dir` in byte order of their names; none when it cannot
/// be listed (absent, not a directory).
fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<PathBuf> = listing.flatten().map(|entry| entry.path()).collect();
    // `OsStr` orders by its encoded bytes: byte order on unix, WTF-8 order
    // on Windows, the same on both snapshots either way.
    entries.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    entries
}

/// One `Tree` root's walk: the files found so far, at most [`CAP`].
struct Walker<'a> {
    skip: &'a [String],
    read: Reader<'a>,
    found: BTreeMap<PathBuf, State>,
}

impl Walker<'_> {
    /// Record the files under `dir`; false once [`CAP`] are recorded and
    /// more remain. Directories are entered, symlinks are not.
    fn walk(&mut self, dir: &Path) -> bool {
        for path in sorted_entries(dir) {
            let skipped = path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                self.skip
                    .iter()
                    .any(|prefix| name.starts_with(prefix.as_str()))
            });
            if skipped {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                if !self.walk(&path) {
                    return false;
                }
                continue;
            }
            if self.found.len() == CAP {
                return false;
            }
            if let Some(state) = state_of(&path, self.read) {
                self.found.insert(path, state);
            }
        }
        true
    }
}

/// One path's state; `None` when it is absent or is neither a regular file
/// nor a symlink (a FIFO, a socket, a device, a directory).
fn state_of(path: &Path, read: Reader<'_>) -> Option<State> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(_) => return Some(State::Unreadable),
    };
    let kind = meta.file_type();
    if kind.is_symlink() {
        return Some(match std::fs::read_link(path) {
            Ok(target) => State::Link(hash(target.as_os_str().as_encoded_bytes())),
            Err(_) => State::Unreadable,
        });
    }
    if !kind.is_file() {
        return None;
    }
    if meta.len() > HASH_LIMIT {
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok());
        return Some(match modified {
            Some(since) => State::Large {
                modified_ns: since.as_nanos(),
                size: meta.len(),
                mode: mode_of(&meta),
            },
            None => State::Unreadable,
        });
    }
    match read(path) {
        Ok(bytes) => Some(State::File {
            sha: hash(&bytes),
            size: bytes.len() as u64,
            mode: mode_of(&meta),
        }),
        Err(_) => Some(State::Unreadable),
    }
}

fn hash(bytes: &[u8]) -> String {
    crate::sha256::hex(&crate::sha256::sha256(bytes))
}

#[cfg(unix)]
fn mode_of(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn mode_of(_meta: &std::fs::Metadata) -> u32 {
    0
}

/// What changed from `before` to `after`, one entry per path, in path
/// order. A path unreadable on both sides says nothing: it was never
/// watched. A truncated directory is named only beside a real change, so a
/// large `~/.ssh` does not speak on every test run.
pub fn diff(before: &Snapshot, after: &Snapshot) -> Vec<(PathBuf, Change)> {
    let mut changes = Vec::new();
    let paths: BTreeSet<&PathBuf> = before.paths.keys().chain(after.paths.keys()).collect();
    for path in paths {
        let change = match (before.paths.get(path), after.paths.get(path)) {
            (None, Some(State::Unreadable)) | (Some(State::Unreadable), None) => None,
            (None, Some(_)) => Some(Change::Added),
            (Some(_), None) => Some(Change::Removed),
            (Some(State::Unreadable), Some(State::Unreadable)) => None,
            (Some(State::Unreadable), Some(_)) | (Some(_), Some(State::Unreadable)) => {
                Some(Change::Unreadable)
            }
            (Some(was), Some(now)) if was != now => Some(Change::Modified),
            (Some(_), Some(_)) | (None, None) => None,
        };
        if let Some(change) = change {
            changes.push((path.clone(), change));
        }
    }
    if changes.iter().any(|(_, change)| change.is_real()) {
        let cut: BTreeSet<&PathBuf> = before.truncated.iter().chain(&after.truncated).collect();
        changes.extend(cut.into_iter().map(|dir| (dir.clone(), Change::Truncated)));
    }
    changes
}

/// How a path prints: `~/…` under `home` on unix, absolute otherwise (a
/// Windows path, or one outside home), so it can be pasted.
pub fn display(path: &Path, home: Option<&Path>) -> String {
    if cfg!(unix) {
        if let Some(rest) = home.and_then(|home| path.strip_prefix(home).ok()) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

// The record kept between PreToolUse and PostToolUse.

/// A path as one record field: `None` when it is not UTF-8 or holds a tab or
/// a newline, which would break the one-line-per-entry format.
fn field(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    (!text.contains(['\t', '\n', '\r'])).then(|| text.to_string())
}

/// The snapshot as text: a [`HEADER`] line, then one line per root, per
/// truncated root and per path. A path [`field`] refuses is left out.
pub fn encode(snapshot: &Snapshot) -> String {
    let mut out = format!("{HEADER}\n");
    for root in &snapshot.roots {
        let line = match root {
            Root::File(path) => field(path).map(|path| format!("root\tfile\t{path}")),
            Root::Tree { dir, skip } => {
                field(dir).map(|dir| format!("root\ttree\t{dir}\t{}", skip.join("\t")))
            }
            Root::Glob {
                dir,
                prefix,
                suffix,
            } => field(dir).map(|dir| format!("root\tglob\t{dir}\t{prefix}\t{suffix}")),
        };
        if let Some(line) = line {
            out.push_str(&line);
            out.push('\n');
        }
    }
    for dir in snapshot.truncated.iter().filter_map(|dir| field(dir)) {
        out.push_str(&format!("cut\t{dir}\n"));
    }
    for (path, state) in &snapshot.paths {
        let Some(path) = field(path) else { continue };
        let state = match state {
            State::File { sha, size, mode } => format!("file\t{sha}\t{size}\t{mode}"),
            State::Large {
                modified_ns,
                size,
                mode,
            } => format!("large\t{modified_ns}\t{size}\t{mode}"),
            State::Link(target) => format!("link\t{target}"),
            State::Unreadable => "unreadable".to_string(),
        };
        out.push_str(&format!("path\t{path}\t{state}\n"));
    }
    out
}

/// `snapshot` as it reads back from its record: what [`encode`] leaves out
/// is left out here too, so the snapshot after a command compares like for
/// like with the one kept before it and such a file never shows as added.
pub fn as_recorded(snapshot: &Snapshot) -> Snapshot {
    decode_body(encode(snapshot).lines().skip(1))
}

/// [`encode`] read back; `None` without this version's [`HEADER`].
pub fn decode(text: &str) -> Option<Snapshot> {
    let mut lines = text.lines();
    (lines.next() == Some(HEADER)).then(|| decode_body(lines))
}

/// The lines after the header; a line it does not know is skipped.
fn decode_body<'a>(lines: impl Iterator<Item = &'a str>) -> Snapshot {
    let mut snapshot = Snapshot::default();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["root", "file", path] => snapshot.roots.push(Root::File(PathBuf::from(path))),
            ["root", "tree", dir, skip @ ..] => snapshot.roots.push(Root::Tree {
                dir: PathBuf::from(dir),
                skip: skip
                    .iter()
                    .filter(|prefix| !prefix.is_empty())
                    .map(|prefix| prefix.to_string())
                    .collect(),
            }),
            ["root", "glob", dir, prefix, suffix] => snapshot.roots.push(Root::Glob {
                dir: PathBuf::from(dir),
                prefix: prefix.to_string(),
                suffix: suffix.to_string(),
            }),
            ["cut", dir] => snapshot.truncated.push(PathBuf::from(dir)),
            ["path", path, "file", sha, size, mode] => {
                let (Ok(size), Ok(mode)) = (size.parse(), mode.parse()) else {
                    continue;
                };
                snapshot.paths.insert(
                    PathBuf::from(path),
                    State::File {
                        sha: sha.to_string(),
                        size,
                        mode,
                    },
                );
            }
            ["path", path, "large", modified_ns, size, mode] => {
                let (Ok(modified_ns), Ok(size), Ok(mode)) =
                    (modified_ns.parse(), size.parse(), mode.parse())
                else {
                    continue;
                };
                snapshot.paths.insert(
                    PathBuf::from(path),
                    State::Large {
                        modified_ns,
                        size,
                        mode,
                    },
                );
            }
            ["path", path, "link", target] => {
                snapshot
                    .paths
                    .insert(PathBuf::from(path), State::Link(target.to_string()));
            }
            ["path", path, "unreadable"] => {
                snapshot
                    .paths
                    .insert(PathBuf::from(path), State::Unreadable);
            }
            _ => {}
        }
    }
    snapshot
}

/// How long a record nobody came back for is kept: an interrupted command
/// gets no PostToolUse.
const KEEP: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Where the records live: `previews/home` under the journal's directory.
pub fn store() -> Option<PathBuf> {
    Some(crate::push_record::dir()?.join("home"))
}

/// A `tool_use_id` checked to name one file in the store and nothing else:
/// no `/`, `\`, `:` or `..`, and not empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordId(String);

impl TryFrom<&str> for RecordId {
    type Error = &'static str;

    fn try_from(id: &str) -> Result<Self, Self::Error> {
        if id.is_empty() || id.contains(['/', '\\', ':']) || id.contains("..") {
            Err("the tool_use_id names no single file")
        } else {
            Ok(RecordId(id.to_string()))
        }
    }
}

/// Keep `snapshot` for this call, after pruning the store's stale records.
/// The error names the path it could not write.
pub fn keep(store: &Path, id: &RecordId, snapshot: &Snapshot) -> io::Result<()> {
    let path = store.join(&id.0);
    let named = |e: io::Error| io::Error::new(e.kind(), format!("{}: {e}", path.display()));
    std::fs::create_dir_all(store).map_err(named)?;
    crate::journal::private(store, 0o700);
    prune(store);
    crate::atomic::write_atomic(&path, &encode(snapshot)).map_err(named)?;
    crate::journal::private(&path, 0o600);
    Ok(())
}

/// What [`take`] found for one call.
#[derive(Debug, PartialEq, Eq)]
pub enum Taken {
    Kept(Snapshot),
    /// No record: the PreToolUse kept none, or it could not be read.
    Absent,
    /// A record without this version's [`HEADER`].
    OtherVersion,
}

/// Read and remove the record kept for this call.
pub fn take(store: &Path, id: &RecordId) -> Taken {
    let path = store.join(&id.0);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Taken::Absent;
    };
    let _ = std::fs::remove_file(&path); // best-effort: a leftover is pruned after a day
    match decode(&text) {
        Some(snapshot) => Taken::Kept(snapshot),
        None => Taken::OtherVersion,
    }
}

fn prune(store: &Path) {
    let Ok(entries) = std::fs::read_dir(store) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_file(entry.path()); // best-effort: retried at the next record
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch home, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!(
                "gb-homeprint-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|since| since.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn join(&self, rest: impl AsRef<Path>) -> PathBuf {
            self.0.join(rest)
        }

        fn roots(&self, git_dir: Option<&Path>) -> Vec<Root> {
            let home = self.0.as_os_str().to_os_string();
            watched(&move |key| (key == "HOME").then(|| home.clone()), git_dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0); // best-effort: temp is cleaned eventually
        }
    }

    #[test]
    fn no_change_is_no_drift() {
        let home = Scratch::new("same");
        std::fs::write(home.join(".gitconfig"), "[user]\n").unwrap();
        let roots = home.roots(None);
        assert!(diff(&snapshot(&roots), &snapshot(&roots)).is_empty());
    }

    #[test]
    fn modified_added_and_removed_each_show() {
        let home = Scratch::new("mar");
        std::fs::write(home.join(".gitconfig"), "a").unwrap();
        std::fs::write(home.join(".netrc"), "x").unwrap();
        let roots = home.roots(None);
        let before = snapshot(&roots);
        std::fs::write(home.join(".gitconfig"), "b").unwrap();
        std::fs::remove_file(home.join(".netrc")).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(home.join(".claude").join("settings.local.json"), "{}").unwrap();
        assert_eq!(
            diff(&before, &snapshot(&roots)),
            vec![
                (home.join(".claude/settings.local.json"), Change::Added),
                (home.join(".gitconfig"), Change::Modified),
                (home.join(".netrc"), Change::Removed),
            ]
        );
    }

    #[test]
    fn the_glob_takes_settings_json_only() {
        let home = Scratch::new("glob");
        let claude = home.join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        let roots = home.roots(None);
        let before = snapshot(&roots);
        std::fs::write(claude.join("history.jsonl"), "x").unwrap();
        std::fs::write(claude.join("settings.json.bak"), "x").unwrap();
        assert!(diff(&before, &snapshot(&roots)).is_empty());
    }

    #[test]
    fn known_hosts_is_not_watched_but_the_rest_of_ssh_is() {
        let home = Scratch::new("ssh");
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(ssh.join("conf.d")).unwrap();
        let roots = home.roots(None);
        let before = snapshot(&roots);
        std::fs::write(ssh.join("known_hosts"), "host").unwrap();
        std::fs::write(ssh.join("known_hosts.old"), "host").unwrap();
        assert!(diff(&before, &snapshot(&roots)).is_empty());
        std::fs::write(ssh.join("conf.d").join("x"), "Host *").unwrap();
        assert_eq!(
            diff(&before, &snapshot(&roots)),
            vec![(ssh.join("conf.d").join("x"), Change::Added)]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_mode_change_is_a_modification() {
        use std::os::unix::fs::PermissionsExt;
        let home = Scratch::new("mode");
        let netrc = home.join(".netrc");
        std::fs::write(&netrc, "x").unwrap();
        std::fs::set_permissions(&netrc, std::fs::Permissions::from_mode(0o600)).unwrap();
        let roots = home.roots(None);
        let before = snapshot(&roots);
        std::fs::set_permissions(&netrc, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            diff(&before, &snapshot(&roots)),
            vec![(netrc, Change::Modified)]
        );
    }

    #[test]
    fn a_read_error_is_unreadable_and_silent_when_on_both_sides() {
        let home = Scratch::new("unread");
        std::fs::write(home.join(".gitconfig"), "a").unwrap();
        let roots = home.roots(None);
        let fail: Reader<'_> = &|_| Err(io::Error::other("injected"));
        let before = snapshot_with(&roots, fail);
        assert_eq!(
            before.paths.get(&home.join(".gitconfig")),
            Some(&State::Unreadable)
        );
        assert!(diff(&before, &snapshot_with(&roots, fail)).is_empty());
        assert_eq!(
            diff(&before, &snapshot(&roots)),
            vec![(home.join(".gitconfig"), Change::Unreadable)]
        );
    }

    /// A file over the limit is never read, and a rewrite of it still shows.
    #[test]
    fn a_large_file_is_fingerprinted_without_being_read() {
        let home = Scratch::new("large");
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        let big = ssh.join("archive");
        let size = usize::try_from(HASH_LIMIT).unwrap() + 1;
        std::fs::write(&big, vec![b'a'; size]).unwrap();
        let roots = home.roots(None);
        let never: Reader<'_> = &|path| panic!("read {}", path.display());
        let before = snapshot_with(&roots, never);
        assert!(matches!(before.paths.get(&big), Some(State::Large { .. })));
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&big)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(
            diff(&before, &snapshot_with(&roots, never)),
            vec![(big, Change::Modified)]
        );
    }

    #[test]
    fn truncation_is_stable_and_named_only_beside_a_change() {
        let home = Scratch::new("cap");
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        for i in 0..600 {
            std::fs::write(ssh.join(format!("k{i:04}")), "x").unwrap();
        }
        let roots = home.roots(None);
        let first = snapshot(&roots);
        let second = snapshot(&roots);
        assert_eq!(first.truncated, vec![ssh.clone()]);
        assert_eq!(first.paths.len(), CAP);
        assert!(diff(&first, &second).is_empty());
        std::fs::write(home.join(".gitconfig"), "new").unwrap();
        assert_eq!(
            diff(&first, &snapshot(&roots)),
            vec![
                (home.join(".gitconfig"), Change::Added),
                (ssh, Change::Truncated)
            ]
        );
    }

    #[test]
    fn the_truncated_note_names_the_cap() {
        assert!(Change::Truncated.as_str().contains(&CAP.to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_skipped_and_does_not_hang() {
        let home = Scratch::new("fifo");
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        let fifo = ssh.join("ctl");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|status| status.success());
        assert!(made, "mkfifo");
        let taken = snapshot(&home.roots(None));
        assert!(!taken.paths.contains_key(&fifo));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_recorded_by_target_not_followed() {
        let home = Scratch::new("link");
        let target = home.join("elsewhere");
        std::fs::write(&target, "one").unwrap();
        std::os::unix::fs::symlink(&target, home.join(".gitconfig")).unwrap();
        let roots = home.roots(None);
        let before = snapshot(&roots);
        assert!(matches!(
            before.paths.get(&home.join(".gitconfig")),
            Some(State::Link(_))
        ));
        std::fs::write(&target, "two").unwrap();
        assert!(diff(&before, &snapshot(&roots)).is_empty());
    }

    #[test]
    fn userprofile_stands_in_for_home_and_xdg_moves_git_config() {
        let var = |key: &str| match key {
            "USERPROFILE" => Some(OsString::from("/u")),
            "XDG_CONFIG_HOME" => Some(OsString::from("/x")),
            "CLAUDE_CONFIG_DIR" => Some(OsString::from("/c")),
            _ => None,
        };
        let roots = watched(&var, Some(Path::new("/r/.git")));
        assert!(roots.contains(&Root::File(PathBuf::from("/u").join(".gitconfig"))));
        assert!(roots.contains(&Root::File(PathBuf::from("/x").join("git").join("config"))));
        assert!(roots.contains(&Root::File(
            PathBuf::from("/c")
                .join("plugins")
                .join("installed_plugins.json")
        )));
        assert!(roots.contains(&Root::File(PathBuf::from("/r/.git").join("config"))));
    }

    #[test]
    fn an_empty_home_falls_back_to_userprofile() {
        let var = |key: &str| match key {
            "HOME" => Some(OsString::new()),
            "USERPROFILE" => Some(OsString::from("/u")),
            _ => None,
        };
        assert_eq!(home(&var), Some(PathBuf::from("/u")));
        assert!(watched(&var, None).contains(&Root::File(PathBuf::from("/u").join(".gitconfig"))));
    }

    fn git(args: &[&str], dir: &Path) {
        let ran = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        assert!(ran, "git {args:?}");
    }

    /// Agrees with `git rev-parse --git-common-dir` in a main checkout, a
    /// subdirectory of it and a linked worktree; outside a repository, none.
    #[test]
    fn the_common_git_dir_is_the_one_git_reports() {
        let root = Scratch::new("gitdir");
        let main = root.join("main");
        std::fs::create_dir_all(main.join("sub")).unwrap();
        git(&["init", "-q", "--template="], &main);
        git(
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "x",
            ],
            &main,
        );
        git(&["worktree", "add", "-q", "--detach", "../wt"], &main);
        // Compared as the same directory, not the same spelling: on Windows
        // the answer keeps the `absolute` form (an 8.3 name such as
        // `RUNNER~1`), where `canonicalize` gives `\\?\` and the long name.
        let same_dir =
            |found: Option<PathBuf>| found.map(|dir| std::fs::canonicalize(dir).unwrap());
        let want = std::fs::canonicalize(main.join(".git")).unwrap();
        assert_eq!(same_dir(git_common_dir(&main)), Some(want.clone()));
        assert_eq!(
            same_dir(git_common_dir(&main.join("sub"))),
            Some(want.clone())
        );
        assert_eq!(same_dir(git_common_dir(&root.join("wt"))), Some(want));
        assert!(git_common_dir(&main).is_some_and(|dir| dir.is_absolute()));
        let outside = Scratch::new("nogit");
        if outside.0.ancestors().all(|dir| !dir.join(".git").exists()) {
            assert_eq!(git_common_dir(&outside.0), None);
        }
    }

    /// The format itself, written out by hand: a round trip alone cannot
    /// show that either direction writes the intended lines.
    #[test]
    fn a_record_has_the_header_then_one_line_per_root_and_path() {
        let snapshot = Snapshot {
            roots: vec![
                Root::File(PathBuf::from("/h/.gitconfig")),
                Root::Tree {
                    dir: PathBuf::from("/h/.ssh"),
                    skip: vec!["known_hosts".to_string()],
                },
                Root::Glob {
                    dir: PathBuf::from("/h/.claude"),
                    prefix: "settings".to_string(),
                    suffix: ".json".to_string(),
                },
            ],
            paths: BTreeMap::from([
                (
                    PathBuf::from("/h/.gitconfig"),
                    State::File {
                        sha: "ab".to_string(),
                        size: 3,
                        mode: 420,
                    },
                ),
                (PathBuf::from("/h/.ssh/l"), State::Link("cd".to_string())),
                (
                    PathBuf::from("/h/.ssh/m"),
                    State::Large {
                        modified_ns: 7,
                        size: 2_000_000,
                        mode: 384,
                    },
                ),
                (PathBuf::from("/h/.ssh/u"), State::Unreadable),
            ]),
            truncated: vec![PathBuf::from("/h/.ssh")],
        };
        let want = "homeprint/1\n\
                    root\tfile\t/h/.gitconfig\n\
                    root\ttree\t/h/.ssh\tknown_hosts\n\
                    root\tglob\t/h/.claude\tsettings\t.json\n\
                    cut\t/h/.ssh\n\
                    path\t/h/.gitconfig\tfile\tab\t3\t420\n\
                    path\t/h/.ssh/l\tlink\tcd\n\
                    path\t/h/.ssh/m\tlarge\t7\t2000000\t384\n\
                    path\t/h/.ssh/u\tunreadable\n";
        assert_eq!(encode(&snapshot), want);
        assert_eq!(decode(want), Some(snapshot));
    }

    #[test]
    fn a_record_without_this_header_is_not_read() {
        assert_eq!(decode("root\tfile\t/h/.gitconfig\n"), None);
        assert_eq!(decode("homeprint/2\nroot\tfile\t/h/.gitconfig\n"), None);
    }

    #[test]
    fn a_real_snapshot_round_trips() {
        let home = Scratch::new("codec");
        std::fs::write(home.join(".gitconfig"), "a").unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh").join("id"), "k").unwrap();
        let taken = snapshot(&home.roots(Some(&home.join("repo/.git"))));
        assert_eq!(decode(&encode(&taken)), Some(taken));
    }

    #[cfg(unix)]
    #[test]
    fn a_name_the_record_cannot_hold_is_never_added() {
        let home = Scratch::new("oddname");
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::write(ssh.join("bad\nname"), "k").unwrap();
        std::fs::write(ssh.join("tab\there"), "k").unwrap();
        let roots = home.roots(None);
        let kept = decode(&encode(&snapshot(&roots))).unwrap();
        assert!(diff(&kept, &as_recorded(&snapshot(&roots))).is_empty());
    }

    #[test]
    fn keep_then_take_once() {
        let home = Scratch::new("store");
        let store = home.join("home");
        let taken = snapshot(&home.roots(None));
        let id = RecordId::try_from("toolu_1").unwrap();
        keep(&store, &id, &taken).unwrap();
        assert_eq!(take(&store, &id), Taken::Kept(taken));
        assert_eq!(take(&store, &id), Taken::Absent);
    }

    #[test]
    fn a_record_another_version_wrote_is_taken_as_such_and_removed() {
        let home = Scratch::new("otherver");
        let store = home.join("home");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("t"), "homeprint/0\nroot\tfile\t/h/x\n").unwrap();
        let id = RecordId::try_from("t").unwrap();
        assert_eq!(take(&store, &id), Taken::OtherVersion);
        assert_eq!(take(&store, &id), Taken::Absent);
    }

    #[test]
    fn an_id_naming_more_than_one_file_is_refused() {
        for bad in ["", "a/b", "a\\b", "c:d", "..", "x..y"] {
            assert!(RecordId::try_from(bad).is_err(), "{bad}");
        }
        assert!(RecordId::try_from("toolu_01Ab-9").is_ok());
    }

    #[test]
    fn a_failed_keep_names_the_path() {
        let home = Scratch::new("keepfail");
        let blocker = home.join("file");
        std::fs::write(&blocker, "x").unwrap();
        let id = RecordId::try_from("t").unwrap();
        let error = keep(&blocker.join("store"), &id, &Snapshot::default()).unwrap_err();
        assert!(error.to_string().contains("store"), "{error}");
    }

    /// The hook's own cost on this machine's real home, read-only: 100 runs
    /// of what one side of the hook does. Run by hand for the budget
    /// (`cargo test --release -p greenbelay-guard --lib
    /// fingerprint_cost -- --ignored --nocapture`); the budget is 20 ms p95.
    #[test]
    #[ignore = "reads the real home; a measurement, not a check"]
    fn fingerprint_cost_on_this_machine() {
        let cwd = std::env::current_dir().unwrap();
        let mut costs: Vec<std::time::Duration> = (0..100)
            .map(|_| {
                let started = std::time::Instant::now();
                let git_dir = git_common_dir(&cwd);
                let roots = watched(&|key| std::env::var_os(key), git_dir.as_deref());
                let taken = as_recorded(&snapshot(&roots));
                assert!(!encode(&taken).is_empty());
                started.elapsed()
            })
            .collect();
        costs.sort();
        eprintln!(
            "fingerprint: p50 {:?} p95 {:?} max {:?}",
            costs[49], costs[94], costs[99]
        );
    }

    #[test]
    fn a_path_under_home_prints_with_a_tilde_on_unix() {
        let shown = display(Path::new("/h/.gitconfig"), Some(Path::new("/h")));
        if cfg!(unix) {
            assert_eq!(shown, "~/.gitconfig");
        } else {
            assert_eq!(shown, Path::new("/h/.gitconfig").display().to_string());
        }
        assert_eq!(
            display(Path::new("/r/.git/config"), Some(Path::new("/h"))),
            Path::new("/r/.git/config").display().to_string()
        );
    }
}
