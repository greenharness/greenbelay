//! Which session published which commit: `<data dir>/pushes.log`, one line
//! per push that left the machine, and the launcher of each detached land.
//!
//! Exit 0 is not publication. A `git push` is credited only for a ref-update
//! line in its output (`<old>..<new>  <src> -> <dst>`, `* [new branch]`, a
//! forced `+ <old>...<new>`), and a land only for a receipt whose status is
//! `landed`. "Everything up-to-date" credits no one.
//!
//! A line: `<ts>\t<session>\t<name|->\t<pid>\t<started_at>\t<repo_identity>\t<branch>\t<sha>`.
//! A tab or newline inside a field becomes a space. Each line is one
//! `O_APPEND` write; past 1 MB the log moves to `pushes.log.1` under its
//! lock, so a reader sees at most two files.

use super::{Identity, Store};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const ROTATE_AT: u64 = 1024 * 1024;
const LOCK_BUDGET: Duration = Duration::from_millis(500);

/// One credited push.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pushed {
    pub at: u64,
    pub identity: Identity,
    pub repo_identity: PathBuf,
    pub branch: String,
    pub sha: String,
}

/// One ref-update line of `git push` output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefUpdate {
    /// The new sha as printed (abbreviated); `None` for a new branch.
    pub new: Option<String>,
    pub src: String,
    pub dst: String,
}

/// `<data dir>/pushes.log`, beside the store's `owners/`.
pub fn log_path(store: &Store) -> PathBuf {
    store
        .root
        .parent()
        .unwrap_or(&store.root)
        .join("pushes.log")
}

fn launches_dir(store: &Store) -> PathBuf {
    store.root.parent().unwrap_or(&store.root).join("launches")
}

/// The ref updates a `git push` printed (git writes them to stderr). A
/// rejected (`!`) or up-to-date (`=`) line is not an update.
pub fn ref_updates(output: &str) -> Vec<RefUpdate> {
    let mut found = Vec::new();
    for line in output.lines() {
        let line = line.trim_start();
        let (flag, rest) = match line.chars().next() {
            Some(flag @ ('*' | '+' | '-' | ' ')) => (flag, line[1..].trim_start()),
            _ => ('.', line),
        };
        let Some((left, dst)) = rest.split_once(" -> ") else {
            continue;
        };
        let dst = dst.split_whitespace().next().unwrap_or("");
        let mut words = left.split_whitespace();
        let (Some(range), Some(src)) = (words.next(), words.next()) else {
            continue;
        };
        if dst.is_empty() || words.next().is_some() {
            continue;
        }
        // `* [new branch]` has a two-word flag column and is read below.
        let Some((_, new)) = range.split_once("...").or_else(|| range.split_once("..")) else {
            continue;
        };
        if flag == '-' || !is_hex(new) {
            continue;
        }
        found.push(RefUpdate {
            new: Some(new.to_owned()),
            src: src.to_owned(),
            dst: dst.to_owned(),
        });
    }
    // `* [new branch]` splits as `[new`, `branch]`, `src`: read it again.
    for line in output.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix('*') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest
            .strip_prefix("[new branch]")
            .or_else(|| rest.strip_prefix("[new reference]"))
        else {
            continue;
        };
        let Some((src, dst)) = rest.split_once(" -> ") else {
            continue;
        };
        let (src, dst) = (src.trim(), dst.split_whitespace().next().unwrap_or(""));
        if !src.is_empty() && !dst.is_empty() && !src.contains(char::is_whitespace) {
            found.push(RefUpdate {
                new: None,
                src: src.to_owned(),
                dst: dst.to_owned(),
            });
        }
    }
    found
}

fn is_hex(text: &str) -> bool {
    text.len() >= 4 && text.chars().all(|c| c.is_ascii_hexdigit())
}

fn clean(field: &str) -> String {
    field.replace(['\t', '\n', '\r'], " ")
}

fn clean_name(name: &str) -> String {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        name.to_owned()
    } else {
        "-".to_owned()
    }
}

fn line_of(push: &Pushed) -> String {
    let id = &push.identity;
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        push.at,
        clean(&id.session),
        clean_name(&id.name),
        id.pid,
        id.started_at,
        clean(&push.repo_identity.to_string_lossy()),
        clean(&push.branch),
        clean(&push.sha),
    )
}

fn parse_line(line: &str) -> Option<Pushed> {
    let fields: Vec<&str> = line.split('\t').collect();
    let [at, session, name, pid, started_at, repo, branch, sha] = fields.as_slice() else {
        return None;
    };
    Some(Pushed {
        at: at.parse().ok()?,
        identity: Identity {
            session: (*session).to_owned(),
            pid: pid.parse().ok()?,
            started_at: started_at.parse().ok()?,
            name: if *name == "-" {
                String::new()
            } else {
                (*name).to_owned()
            },
            reference: None,
        },
        repo_identity: PathBuf::from(repo),
        branch: (*branch).to_owned(),
        sha: (*sha).to_owned(),
    })
}

/// Append one credited push. Past [`ROTATE_AT`] the log is moved aside
/// first, under its lock; a busy lock skips the rotation, never the line.
pub fn append(store: &Store, push: &Pushed) -> std::io::Result<()> {
    let path = log_path(store);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() >= ROTATE_AT) {
        let mut lock = path.as_os_str().to_os_string();
        lock.push(".lock");
        if let Ok(Some(_guard)) = crate::lock::lock_within(Path::new(&lock), LOCK_BUDGET) {
            if std::fs::metadata(&path).is_ok_and(|meta| meta.len() >= ROTATE_AT) {
                // A failed move skips this rotation, never the line.
                if let Err(failure) = std::fs::rename(&path, path.with_extension("log.1")) {
                    eprintln!(
                        "{}: pushes.log: could not rotate {}: {failure}",
                        crate::bin!(),
                        path.display()
                    );
                }
            }
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line_of(push).as_bytes())
}

/// Every recorded push, oldest first: the rotated file, then the log.
pub fn read_all(store: &Store) -> Vec<Pushed> {
    let path = log_path(store);
    [path.with_extension("log.1"), path]
        .iter()
        .filter_map(|file| std::fs::read_to_string(file).ok())
        .flat_map(|text| text.lines().filter_map(parse_line).collect::<Vec<Pushed>>())
        .collect()
}

/// The newest push of `branch` in the repository `repo_identity`.
pub fn latest_of(store: &Store, repo_identity: &Path, branch: &str) -> Option<Pushed> {
    read_all(store)
        .into_iter()
        .rev()
        .find(|push| push.branch == branch && push.repo_identity == repo_identity)
}

fn launch_file(store: &Store, receipt: &str) -> PathBuf {
    launches_dir(store).join(format!("{:016x}.json", super::fnv1a64(receipt.as_bytes())))
}

/// Remember who launched the detached land whose receipt is `receipt`, and
/// in which repository: the session that waits on it may be elsewhere.
pub fn save_launcher(
    store: &Store,
    receipt: &str,
    identity: &Identity,
    repo_identity: &Path,
) -> std::io::Result<()> {
    let path = launch_file(store, receipt);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::json!({
        "receipt": receipt,
        "session": identity.session,
        "pid": identity.pid,
        "started_at": identity.started_at,
        "name": identity.name,
        "repo_identity": repo_identity.to_string_lossy(),
    });
    crate::atomic::write_atomic(&path, &body.to_string())
}

/// The launcher of `receipt`'s land. `None` when no launch was recorded, or
/// a wait already credited it ([`forget_launcher`]).
pub fn launcher(store: &Store, receipt: &str) -> Option<(Identity, PathBuf)> {
    let path = launch_file(store, receipt);
    let text = std::fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    if value.get("receipt")?.as_str()? != receipt {
        return None;
    }
    let identity = Identity {
        session: value.get("session")?.as_str()?.to_owned(),
        pid: u32::try_from(value.get("pid")?.as_u64()?).ok()?,
        started_at: value.get("started_at")?.as_u64()?,
        name: value
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or_default()
            .to_owned(),
        reference: None,
    };
    let repo = PathBuf::from(value.get("repo_identity")?.as_str()?);
    Some((identity, repo))
}

/// Drop `receipt`'s launcher once its push is in the log, so a second wait
/// on the same receipt credits no one again.
pub fn forget_launcher(store: &Store, receipt: &str) -> std::io::Result<()> {
    match std::fs::remove_file(launch_file(store, receipt)) {
        Err(failure) if failure.kind() != std::io::ErrorKind::NotFound => Err(failure),
        _ => Ok(()),
    }
}

/// What one Bash call published, credited: one journal line each. `caller`
/// answers who ran it; it is asked only when something is to be credited.
pub fn credit_after_bash(
    store: &Store,
    bash: &crate::payload::Bash,
    now: u64,
    caller: impl Fn() -> Option<Identity>,
) -> Vec<String> {
    if bash.errored || bash.background || !bash.has_response {
        return Vec::new();
    }
    let parsed = crate::shell::lex(&bash.command);
    let context = crate::rules::Context {
        cwd: &bash.cwd,
        parsed: &parsed,
        background: false,
        timeout_ms: None,
        tool_use_id: &bash.tool_use_id,
        transcript: None,
        agent_transcript: None,
        command: &bash.command,
        platform: None,
        output: None,
        session: &bash.session,
        failed: false,
    };
    let output = format!(
        "{}\n{}",
        bash.stderr.as_deref().unwrap_or_default(),
        bash.stdout.as_deref().unwrap_or_default()
    );
    let mut said = Vec::new();
    // `true` once the push is in the log, written now or already there (a
    // hook run twice for one call, or two waits on one receipt).
    let mut credit = |identity: Identity, repo_identity: PathBuf, branch: String, sha: String| {
        let shown = format!("pushed {branch} {sha}");
        let already = read_all(store).last().is_some_and(|newest| {
            newest.identity.session == identity.session
                && newest.branch == branch
                && newest.sha == sha
        });
        if already {
            return true;
        }
        let push = Pushed {
            at: now,
            identity,
            repo_identity,
            branch,
            sha,
        };
        match append(store, &push) {
            Ok(()) => {
                said.push(shown);
                true
            }
            Err(failure) => {
                said.push(format!("error: {}: {failure}", log_path(store).display()));
                false
            }
        }
    };
    if let Some(push) = read_push(&parsed) {
        let Some(dir) = push_dir(&parsed, &context, push.at) else {
            return said;
        };
        let updates: Vec<RefUpdate> = ref_updates(&output)
            .into_iter()
            .filter(|update| {
                push.branch.is_none_or(|branch| {
                    short(&update.dst) == short(branch) || short(&update.src) == short(branch)
                })
            })
            .collect();
        let [update] = updates.as_slice() else {
            return said;
        };
        let Some(place) = crate::activity::resolve(&dir) else {
            return said;
        };
        let remote = push.remote.unwrap_or("origin");
        let dst = short(&update.dst).to_owned();
        let sha = match &update.new {
            Some(new) => full_sha(&dir, new).unwrap_or_else(|| new.clone()),
            None => match full_sha(&dir, &format!("refs/remotes/{remote}/{dst}")) {
                Some(sha) => sha,
                None => return said,
            },
        };
        if let Some(identity) = caller() {
            credit(identity, place.repo_identity, dst, sha);
        }
        return said;
    }
    let Some(land) = parsed
        .judgeable()
        .find(|clause| crate::land::is_land(clause))
    else {
        return said;
    };
    if let Some(path) = crate::land::wait_path(land) {
        let path = expand_home(path);
        let landed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| crate::land::read_receipt(&text))
            .filter(|receipt| receipt.status == "landed");
        if let Some(receipt) = landed {
            if let Some((identity, repo)) = launcher(store, &path) {
                if credit(identity, repo, receipt.branch, receipt.pushed) {
                    if let Err(failure) = forget_launcher(store, &path) {
                        said.push(format!("error: launch of {path}: {failure}"));
                    }
                }
            }
        }
        return said;
    }
    let dir = context.cwd_at(land.at);
    let Some(place) = crate::activity::resolve(&dir) else {
        return said;
    };
    if land.has_flag("--detach") {
        if let (Some(path), Some(identity)) = (printed_wait_path(&output), caller()) {
            if let Err(failure) = save_launcher(store, &path, &identity, &place.repo_identity) {
                said.push(format!("error: launch of {path}: {failure}"));
            }
        }
        return said;
    }
    let Some(judged) = crate::push_record::land_judged(&bash.tool_use_id) else {
        return said;
    };
    let found = crate::land::receipts_since(&judged.receipt_dir, &judged.branch, judged.since);
    if let [receipt] = found.as_slice() {
        if receipt.status == "landed" {
            if let Some(identity) = caller() {
                credit(
                    identity,
                    place.repo_identity,
                    receipt.branch.clone(),
                    receipt.pushed.clone(),
                );
            }
        }
    }
    said
}

/// One `git push [<remote> [<branch>]]` the log can credit.
struct Push<'a> {
    remote: Option<&'a str>,
    branch: Option<&'a str>,
    at: usize,
}

/// The shapes `push-landed` judges, refused for the same reasons (a dry run,
/// a flag that changes what is pushed, a `src:dst` or `+` refspec, several
/// refspecs), read from the words after `push` so that a `git -C <dir>`
/// before it is not taken for the remote.
fn read_push(parsed: &crate::shell::Parsed) -> Option<Push<'_>> {
    use crate::assertions::push_landed::REWRITES_THE_QUESTION;
    let crate::shell::Parsed::Clear(clauses) = parsed else {
        return None;
    };
    if !parsed.fully_read() {
        return None;
    }
    let cmd = clauses
        .iter()
        .find(|c| c.program() == Some("git") && c.subcommand() == Some("push"))?;
    if cmd.is_dry_run() || cmd.has_short('n') {
        return None;
    }
    if REWRITES_THE_QUESTION.iter().any(|flag| cmd.has_flag(flag)) {
        return None;
    }
    let args = cmd.args();
    let after = args
        .iter()
        .position(|word| !word.quoted && word.text == "push")?;
    let operands: Vec<&str> = args[after + 1..]
        .iter()
        .map(|word| word.text.as_str())
        .filter(|text| !text.starts_with('-'))
        .collect();
    let (remote, branch) = match operands.as_slice() {
        [] => (None, None),
        [remote] => (Some(*remote), None),
        [remote, refspec] if !refspec.contains(':') && !refspec.starts_with('+') => {
            (Some(*remote), Some(*refspec))
        }
        _ => return None,
    };
    Some(Push {
        remote,
        branch,
        at: cmd.at,
    })
}

/// Where the push at clause `at` runs: the clause's directory, moved by
/// each literal `git -C <dir>` before `push`. `None` when a `-C` is not a
/// literal.
fn push_dir(
    parsed: &crate::shell::Parsed,
    context: &crate::rules::Context,
    at: usize,
) -> Option<PathBuf> {
    use crate::shell::Resolved;
    let clause = parsed.judgeable().find(|clause| clause.at == at)?;
    let mut dir = context.cwd_at(at);
    let args = clause.args();
    let mut index = 0;
    while let Some(word) = args.get(index) {
        let text = word.text.as_str();
        if !text.starts_with('-') || word.quoted {
            break;
        }
        if text == "-C" {
            match args.get(index + 1).map(|w| parsed.word_value(at, w)) {
                Some(Resolved::Text(path)) => dir = crate::rules::resolve(&dir, &path),
                _ => return None,
            }
        }
        let valued = matches!(
            text,
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace"
        );
        index += if valued { 2 } else { 1 };
    }
    Some(dir)
}

fn short(reference: &str) -> &str {
    reference.strip_prefix("refs/heads/").unwrap_or(reference)
}

/// `name` as a full commit sha in `dir`'s repository.
fn full_sha(dir: &Path, name: &str) -> Option<String> {
    let text = crate::git::stdout_in(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{name}^{{commit}}"),
        ],
    )?;
    let sha = text.trim();
    (sha.len() >= 40 && is_hex(sha)).then(|| sha.to_owned())
}

/// The receipt path a detached land's launch printed:
/// `… land --wait '<path>'` (or unquoted), with a leading `~/` expanded.
pub fn printed_wait_path(output: &str) -> Option<String> {
    let at = output.find("land --wait ")?;
    let rest = output[at + "land --wait ".len()..].trim_start();
    let path = if let Some(quoted) = rest.strip_prefix('\'') {
        quoted.split('\'').next()?
    } else {
        rest.split_whitespace().next()?
    };
    (!path.is_empty()).then(|| expand_home(path))
}

/// `~/x` as `$HOME/x`; anything else as it is.
pub fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => Path::new(&home).join(rest).display().to_string(),
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_update_lines_are_read_and_up_to_date_is_not_one() {
        let output = "To example.org:r.git\n   1a2b3c4..5d6e7f8  feat/x -> feat/x\n";
        assert_eq!(
            ref_updates(output),
            vec![RefUpdate {
                new: Some("5d6e7f8".into()),
                src: "feat/x".into(),
                dst: "feat/x".into()
            }]
        );
        let created = "To r\n * [new branch]      feat/y -> feat/y\n";
        assert_eq!(
            ref_updates(created),
            vec![RefUpdate {
                new: None,
                src: "feat/y".into(),
                dst: "feat/y".into()
            }]
        );
        let forced = " + 1a2b3c4...9f9f9f9 feat/z -> feat/z (forced update)\n";
        assert_eq!(ref_updates(forced)[0].new.as_deref(), Some("9f9f9f9"));
        for nothing in [
            "Everything up-to-date\n",
            " ! [rejected]        feat/x -> feat/x (fetch first)\n",
            " = [up to date]      feat/x -> feat/x\n",
            " - [deleted]         feat/x\n",
        ] {
            assert_eq!(ref_updates(nothing), vec![], "{nothing}");
        }
    }

    #[test]
    fn a_printed_wait_path_is_read_quoted_or_bare() {
        assert_eq!(
            printed_wait_path("detached; then: greengate land --wait '/r/a b.json'\n").as_deref(),
            Some("/r/a b.json")
        );
        assert_eq!(
            printed_wait_path("amont land --wait /r/1-2.json").as_deref(),
            Some("/r/1-2.json")
        );
        assert_eq!(printed_wait_path("landed"), None);
    }

    #[test]
    fn a_field_cannot_break_the_line() {
        let push = Pushed {
            at: 1,
            identity: Identity {
                session: "s\tx".into(),
                pid: 2,
                started_at: 3,
                name: "bad name".into(),
                reference: None,
            },
            repo_identity: PathBuf::from("/r/.git"),
            branch: "feat/x\nfoo".into(),
            sha: "abc".into(),
        };
        let line = line_of(&push);
        assert_eq!(line.matches('\t').count(), 7, "{line}");
        assert_eq!(line.matches('\n').count(), 1, "{line}");
        let back = parse_line(line.trim_end()).unwrap();
        assert_eq!(back.identity.name, "");
        assert_eq!(back.branch, "feat/x foo");
    }
}
