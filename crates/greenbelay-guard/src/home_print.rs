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
//! absent path as absent. Only regular files are opened: a FIFO would block
//! the read and an ssh ControlMaster socket comes and goes, so both are left
//! out. A directory expands to at most [`CAP`] files in byte order; past that
//! it is marked truncated.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// The most files one watched directory expands to.
pub const CAP: usize = 500;

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
    /// A regular file: content hash, size, and mode bits (0 off unix).
    File { sha: String, size: u64, mode: u32 },
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
}

/// The watched set for this environment. `var` reads an environment
/// variable (a closure, so tests never call `set_var`); `git_dir` is the
/// repository's shared git directory, when the command ran in one.
pub fn watched(var: &dyn Fn(&str) -> Option<OsString>, git_dir: Option<&Path>) -> Vec<Root> {
    let set = |k: &str| var(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let mut roots = Vec::new();
    if let Some(home) = set("HOME").or_else(|| set("USERPROFILE")) {
        roots.push(Root::File(home.join(".gitconfig")));
        roots.push(Root::File(home.join(".netrc")));
        if cfg!(windows) {
            roots.push(Root::File(home.join("_netrc")));
        }
        roots.push(Root::Tree {
            dir: home.join(".ssh"),
            skip: vec!["known_hosts".to_string()],
        });
        let xdg = set("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        roots.push(Root::File(xdg.join("git").join("config")));
        let claude = set("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude"));
        roots.push(Root::Glob {
            dir: claude.clone(),
            prefix: "settings".to_string(),
            suffix: ".json".to_string(),
        });
        let plugins = claude.join("plugins");
        roots.push(Root::File(plugins.join("installed_plugins.json")));
        roots.push(Root::File(plugins.join("known_marketplaces.json")));
    } else if let Some(xdg) = set("XDG_CONFIG_HOME") {
        roots.push(Root::File(xdg.join("git").join("config")));
    }
    if let Some(g) = git_dir {
        roots.push(Root::File(g.join("config")));
    }
    roots
}

/// The shared git directory of the repository holding `cwd`, absolute: a
/// linked worktree answers with its main repository's, whose `config` both
/// share. `None` outside a repository.
///
/// Read from disk the way git finds it (the nearest `.git` up from `cwd`; a
/// `.git` file's `gitdir:` line; that directory's `commondir` file), not by
/// spawning `git rev-parse --git-common-dir`: the spawn measured 46 ms p50 on
/// a loaded host, more than the hook's whole 20 ms budget.
pub fn git_common_dir(cwd: &Path) -> Option<PathBuf> {
    let cwd = std::path::absolute(cwd).ok()?;
    let dot_git = cwd
        .ancestors()
        .map(|d| d.join(".git"))
        .find(|g| g.exists())?;
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let target = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
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
    snapshot_with(roots, &|p| std::fs::read(p))
}

/// [`snapshot`] through `read`, the seam a test injects a read error with.
pub fn snapshot_with(roots: &[Root], read: Reader<'_>) -> Snapshot {
    let mut snap = Snapshot {
        roots: roots.to_vec(),
        ..Snapshot::default()
    };
    for root in roots {
        match root {
            Root::File(p) => {
                if let Some(s) = state_of(p, read) {
                    snap.paths.insert(p.clone(), s);
                }
            }
            Root::Tree { dir, skip } => {
                let mut seen = 0usize;
                if !walk(dir, skip, read, &mut snap.paths, &mut seen) {
                    snap.truncated.push(dir.clone());
                }
            }
            Root::Glob {
                dir,
                prefix,
                suffix,
            } => {
                for p in sorted_entries(dir) {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
                    let fits = name.is_some_and(|n| {
                        n.starts_with(prefix.as_str()) && n.ends_with(suffix.as_str())
                    });
                    if fits {
                        if let Some(s) = state_of(&p, read) {
                            snap.paths.insert(p, s);
                        }
                    }
                }
            }
        }
    }
    snap
}

/// The entries of `dir` in byte order of their names; none when it cannot
/// be listed (absent, not a directory).
fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    v.sort_by(|a, b| {
        let a = a.file_name().map(|n| n.as_encoded_bytes().to_vec());
        let b = b.file_name().map(|n| n.as_encoded_bytes().to_vec());
        a.cmp(&b)
    });
    v
}

/// Walk `dir` into `out`; false once [`CAP`] files are recorded and more
/// remain. Directories are entered, symlinks are not.
fn walk(
    dir: &Path,
    skip: &[String],
    read: Reader<'_>,
    out: &mut BTreeMap<PathBuf, State>,
    seen: &mut usize,
) -> bool {
    for p in sorted_entries(dir) {
        let skipped = p.file_name().is_some_and(|n| {
            let n = n.to_string_lossy();
            skip.iter().any(|s| n.starts_with(s.as_str()))
        });
        if skipped {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_dir() {
            if !walk(&p, skip, read, out, seen) {
                return false;
            }
            continue;
        }
        let Some(s) = state_of(&p, read) else {
            continue;
        };
        if *seen == CAP {
            return false;
        }
        *seen += 1;
        out.insert(p, s);
    }
    true
}

/// One path's state; `None` when it is absent or is neither a regular file
/// nor a symlink (a FIFO, a socket, a device, a directory).
fn state_of(p: &Path, read: Reader<'_>) -> Option<State> {
    let meta = match std::fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(_) => return Some(State::Unreadable),
    };
    let kind = meta.file_type();
    if kind.is_symlink() {
        return Some(match std::fs::read_link(p) {
            Ok(t) => State::Link(hash(t.as_os_str().as_encoded_bytes())),
            Err(_) => State::Unreadable,
        });
    }
    if !kind.is_file() {
        return None;
    }
    match read(p) {
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
    let mut out = Vec::new();
    let keys: std::collections::BTreeSet<&PathBuf> =
        before.paths.keys().chain(after.paths.keys()).collect();
    for k in keys {
        let change = match (before.paths.get(k), after.paths.get(k)) {
            (None, Some(State::Unreadable)) | (Some(State::Unreadable), None) => None,
            (None, Some(_)) => Some(Change::Added),
            (Some(_), None) => Some(Change::Removed),
            (Some(State::Unreadable), Some(State::Unreadable)) => None,
            (Some(State::Unreadable), Some(_)) | (Some(_), Some(State::Unreadable)) => {
                Some(Change::Unreadable)
            }
            (Some(a), Some(b)) if a != b => Some(Change::Modified),
            _ => None,
        };
        if let Some(c) = change {
            out.push((k.clone(), c));
        }
    }
    let real = out.iter().any(|(_, c)| *c != Change::Unreadable);
    if real {
        let cut: std::collections::BTreeSet<&PathBuf> =
            before.truncated.iter().chain(&after.truncated).collect();
        out.extend(cut.into_iter().map(|d| (d.clone(), Change::Truncated)));
    }
    out
}

/// How a path prints: `~/…` under `home` on unix, absolute otherwise (a
/// Windows path, or one outside home), so it can be pasted.
pub fn display(path: &Path, home: Option<&Path>) -> String {
    if cfg!(unix) {
        if let Some(rest) = home.and_then(|h| path.strip_prefix(h).ok()) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

// --- the record kept between PreToolUse and PostToolUse -------------------

/// A path that would break the one-line-per-entry format.
fn printable(p: &Path) -> Option<String> {
    let s = p.to_str()?;
    (!s.contains(['\t', '\n', '\r'])).then(|| s.to_string())
}

/// The snapshot as text: one line per root, per truncated root and per path.
/// A path with a tab or a newline in it is left out.
pub fn encode(s: &Snapshot) -> String {
    let mut out = String::new();
    for r in &s.roots {
        let line = match r {
            Root::File(p) => printable(p).map(|p| format!("root\tfile\t{p}")),
            Root::Tree { dir, skip } => {
                printable(dir).map(|d| format!("root\ttree\t{d}\t{}", skip.join("\t")))
            }
            Root::Glob {
                dir,
                prefix,
                suffix,
            } => printable(dir).map(|d| format!("root\tglob\t{d}\t{prefix}\t{suffix}")),
        };
        if let Some(l) = line {
            out.push_str(&l);
            out.push('\n');
        }
    }
    for d in s.truncated.iter().filter_map(|d| printable(d)) {
        out.push_str(&format!("cut\t{d}\n"));
    }
    for (p, st) in &s.paths {
        let Some(p) = printable(p) else { continue };
        let st = match st {
            State::File { sha, size, mode } => format!("file\t{sha}\t{size}\t{mode}"),
            State::Link(t) => format!("link\t{t}"),
            State::Unreadable => "unreadable".to_string(),
        };
        out.push_str(&format!("path\t{p}\t{st}\n"));
    }
    out
}

/// `s` as it reads back from its record: what [`encode`] leaves out (a path
/// that is not UTF-8, or holds a tab or a newline) is left out here too, so
/// the snapshot after a command compares like for like with the one kept
/// before it and such a file never shows as added.
pub fn as_recorded(s: &Snapshot) -> Snapshot {
    decode(&encode(s))
}

/// [`encode`] read back; a line it does not know is skipped.
pub fn decode(text: &str) -> Snapshot {
    let mut s = Snapshot::default();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.as_slice() {
            ["root", "file", p] => s.roots.push(Root::File(PathBuf::from(p))),
            ["root", "tree", d, skip @ ..] => s.roots.push(Root::Tree {
                dir: PathBuf::from(d),
                skip: skip
                    .iter()
                    .filter(|x| !x.is_empty())
                    .map(|x| x.to_string())
                    .collect(),
            }),
            ["root", "glob", d, prefix, suffix] => s.roots.push(Root::Glob {
                dir: PathBuf::from(d),
                prefix: prefix.to_string(),
                suffix: suffix.to_string(),
            }),
            ["cut", d] => s.truncated.push(PathBuf::from(d)),
            ["path", p, "file", sha, size, mode] => {
                let (Ok(size), Ok(mode)) = (size.parse(), mode.parse()) else {
                    continue;
                };
                s.paths.insert(
                    PathBuf::from(p),
                    State::File {
                        sha: sha.to_string(),
                        size,
                        mode,
                    },
                );
            }
            ["path", p, "link", t] => {
                s.paths.insert(PathBuf::from(p), State::Link(t.to_string()));
            }
            ["path", p, "unreadable"] => {
                s.paths.insert(PathBuf::from(p), State::Unreadable);
            }
            _ => {}
        }
    }
    s
}

/// How long a record nobody came back for is kept: an interrupted command
/// gets no PostToolUse.
const KEEP: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Where the records live: `previews/home` under the journal's directory.
pub fn store() -> Option<PathBuf> {
    Some(crate::push_record::dir()?.join("home"))
}

/// An id that names one file in the store, and nothing else.
pub fn usable_id(id: &str) -> bool {
    !id.is_empty() && !id.contains(['/', '\\', ':']) && !id.contains("..")
}

/// Keep `snap` for this call, after pruning the store's stale records.
pub fn keep(store: &Path, tool_use_id: &str, snap: &Snapshot) -> io::Result<()> {
    if !usable_id(tool_use_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the tool_use_id names no single file",
        ));
    }
    std::fs::create_dir_all(store)?;
    crate::journal::private(store, 0o700);
    prune(store);
    let path = store.join(tool_use_id);
    crate::atomic::write_atomic(&path, &encode(snap))?;
    crate::journal::private(&path, 0o600);
    Ok(())
}

/// Read and remove the record kept for this call.
pub fn take(store: &Path, tool_use_id: &str) -> Option<Snapshot> {
    if !usable_id(tool_use_id) {
        return None;
    }
    let path = store.join(tool_use_id);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path); // best-effort: a leftover is pruned after a day
    Some(decode(&text))
}

fn prune(store: &Path) {
    let Ok(entries) = std::fs::read_dir(store) else {
        return;
    };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_file(e.path()); // best-effort: retried at the next record
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "gb-homeprint-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn env_home(home: &Path) -> impl Fn(&str) -> Option<OsString> + '_ {
        move |k| (k == "HOME").then(|| home.as_os_str().to_os_string())
    }

    #[test]
    fn no_change_is_no_drift() {
        let h = scratch("same");
        std::fs::write(h.join(".gitconfig"), "[user]\n").unwrap();
        let roots = watched(&env_home(&h), None);
        assert!(diff(&snapshot(&roots), &snapshot(&roots)).is_empty());
    }

    #[test]
    fn modified_added_and_removed_each_show() {
        let h = scratch("mar");
        std::fs::write(h.join(".gitconfig"), "a").unwrap();
        std::fs::write(h.join(".netrc"), "x").unwrap();
        let roots = watched(&env_home(&h), None);
        let before = snapshot(&roots);
        std::fs::write(h.join(".gitconfig"), "b").unwrap();
        std::fs::remove_file(h.join(".netrc")).unwrap();
        std::fs::create_dir_all(h.join(".claude")).unwrap();
        std::fs::write(h.join(".claude").join("settings.local.json"), "{}").unwrap();
        let got = diff(&before, &snapshot(&roots));
        assert_eq!(
            got,
            vec![
                (h.join(".claude/settings.local.json"), Change::Added),
                (h.join(".gitconfig"), Change::Modified),
                (h.join(".netrc"), Change::Removed),
            ]
        );
    }

    #[test]
    fn the_glob_takes_settings_json_only() {
        let h = scratch("glob");
        let c = h.join(".claude");
        std::fs::create_dir_all(&c).unwrap();
        let roots = watched(&env_home(&h), None);
        let before = snapshot(&roots);
        std::fs::write(c.join("history.jsonl"), "x").unwrap();
        std::fs::write(c.join("settings.json.bak"), "x").unwrap();
        assert!(diff(&before, &snapshot(&roots)).is_empty());
    }

    #[test]
    fn known_hosts_is_not_watched_but_the_rest_of_ssh_is() {
        let h = scratch("ssh");
        let ssh = h.join(".ssh");
        std::fs::create_dir_all(ssh.join("conf.d")).unwrap();
        let roots = watched(&env_home(&h), None);
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

    #[test]
    fn a_mode_change_is_a_modification() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let h = scratch("mode");
            let f = h.join(".netrc");
            std::fs::write(&f, "x").unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
            let roots = watched(&env_home(&h), None);
            let before = snapshot(&roots);
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(
                diff(&before, &snapshot(&roots)),
                vec![(f, Change::Modified)]
            );
        }
    }

    #[test]
    fn a_read_error_is_unreadable_and_silent_when_on_both_sides() {
        let h = scratch("unread");
        std::fs::write(h.join(".gitconfig"), "a").unwrap();
        let roots = watched(&env_home(&h), None);
        let fail: Reader<'_> = &|_| Err(io::Error::other("injected"));
        let before = snapshot_with(&roots, fail);
        assert_eq!(
            before.paths.get(&h.join(".gitconfig")),
            Some(&State::Unreadable)
        );
        assert!(diff(&before, &snapshot_with(&roots, fail)).is_empty());
        assert_eq!(
            diff(&before, &snapshot(&roots)),
            vec![(h.join(".gitconfig"), Change::Unreadable)]
        );
    }

    #[test]
    fn truncation_is_stable_and_named_only_beside_a_change() {
        let h = scratch("cap");
        let ssh = h.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        for i in 0..600 {
            std::fs::write(ssh.join(format!("k{i:04}")), "x").unwrap();
        }
        let roots = watched(&env_home(&h), None);
        let a = snapshot(&roots);
        let b = snapshot(&roots);
        assert_eq!(a.truncated, vec![ssh.clone()]);
        assert_eq!(a.paths.len(), CAP);
        assert!(diff(&a, &b).is_empty());
        std::fs::write(h.join(".gitconfig"), "new").unwrap();
        assert_eq!(
            diff(&a, &snapshot(&roots)),
            vec![
                (h.join(".gitconfig"), Change::Added),
                (ssh, Change::Truncated)
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_skipped_and_does_not_hang() {
        let h = scratch("fifo");
        let ssh = h.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        let fifo = ssh.join("ctl");
        let ok = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "mkfifo");
        let snap = snapshot(&watched(&env_home(&h), None));
        assert!(!snap.paths.contains_key(&fifo));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_recorded_by_target_not_followed() {
        let h = scratch("link");
        let target = h.join("elsewhere");
        std::fs::write(&target, "one").unwrap();
        std::os::unix::fs::symlink(&target, h.join(".gitconfig")).unwrap();
        let roots = watched(&env_home(&h), None);
        let before = snapshot(&roots);
        assert!(matches!(
            before.paths.get(&h.join(".gitconfig")),
            Some(State::Link(_))
        ));
        std::fs::write(&target, "two").unwrap();
        assert!(diff(&before, &snapshot(&roots)).is_empty());
    }

    #[test]
    fn userprofile_stands_in_for_home_and_xdg_moves_git_config() {
        let var = |k: &str| match k {
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
        let var = |k: &str| match k {
            "HOME" => Some(OsString::new()),
            "USERPROFILE" => Some(OsString::from("/u")),
            _ => None,
        };
        assert!(watched(&var, None).contains(&Root::File(PathBuf::from("/u").join(".gitconfig"))));
    }

    fn git(args: &[&str], dir: &Path) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

    /// Agrees with `git rev-parse --git-common-dir` in a main checkout, a
    /// subdirectory of it and a linked worktree; outside a repository, none.
    #[test]
    fn the_common_git_dir_is_the_one_git_reports() {
        let h = scratch("gitdir");
        let main = h.join("main");
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
        let want = std::fs::canonicalize(main.join(".git")).unwrap();
        assert_eq!(git_common_dir(&main), Some(want.clone()));
        assert_eq!(git_common_dir(&main.join("sub")), Some(want.clone()));
        assert_eq!(git_common_dir(&h.join("wt")), Some(want));
        let outside = std::env::temp_dir().join(format!("gb-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        if outside.ancestors().all(|d| !d.join(".git").exists()) {
            assert_eq!(git_common_dir(&outside), None);
        }
    }

    #[test]
    fn the_record_round_trips() {
        let h = scratch("codec");
        std::fs::write(h.join(".gitconfig"), "a").unwrap();
        std::fs::create_dir_all(h.join(".ssh")).unwrap();
        std::fs::write(h.join(".ssh").join("id"), "k").unwrap();
        let snap = snapshot(&watched(&env_home(&h), Some(&h.join("repo/.git"))));
        assert_eq!(decode(&encode(&snap)), snap);
    }

    #[cfg(unix)]
    #[test]
    fn a_name_the_record_cannot_hold_is_never_added() {
        let h = scratch("oddname");
        let ssh = h.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::write(ssh.join("bad\nname"), "k").unwrap();
        std::fs::write(ssh.join("tab\there"), "k").unwrap();
        let roots = watched(&env_home(&h), None);
        let kept = decode(&encode(&snapshot(&roots)));
        assert!(diff(&kept, &as_recorded(&snapshot(&roots))).is_empty());
    }

    #[test]
    fn keep_then_take_once_and_refuse_odd_ids() {
        let h = scratch("store");
        let store = h.join("home");
        let snap = snapshot(&watched(&env_home(&h), None));
        keep(&store, "toolu_1", &snap).unwrap();
        assert_eq!(take(&store, "toolu_1"), Some(snap.clone()));
        assert_eq!(take(&store, "toolu_1"), None);
        for bad in ["", "a/b", "a\\b", "c:d", "..", "x..y"] {
            assert!(keep(&store, bad, &snap).is_err(), "{bad}");
            assert!(take(&store, bad).is_none(), "{bad}");
        }
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
