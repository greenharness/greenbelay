//! Claude Code's session registry: one `<pid>.json` per live session in
//! `~/.claude/sessions`. It is the host's undocumented store, read
//! best-effort and gated on `peerProtocol = 1` (decisions ADR-0039, plan
//! `docs/plans/2026-11-24-one-owner-per-worktree.md`). Anything unexpected is
//! `Unknown`, never `Gone`.
//!
//! The reader opens only files named `*.json`: a `*.key` file holds peer
//! tokens and is never opened.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const REGISTRY_DIR_VAR: &str = "GREENBELAY_HOST_SESSIONS_DIR";
const SUPPORTED_PEER_PROTOCOL: u64 = 1;
/// Replaces the process the caller walk starts from. For tests: the hook and
/// the command run as children of the test, which has no registry file. Read
/// only beside `GREENBELAY_HOST_SESSIONS_DIR`, the test sandbox's registry,
/// so a command cannot pass for another process against the real one.
const CALLER_PID_VAR: &str = crate::env_var!("TEST_CALLER_PID");
/// The caller is at most this many processes up, the starting one included.
const CALLER_LEVELS: u32 = 4;
const PS_BUDGET: Duration = Duration::from_secs(1);

/// One live session as the host registry names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostSession {
    pub session: String,
    pub pid: u32,
    pub started_at: u64,
    pub name: String,
}

/// What an owner record remembers about a session, to be compared with the
/// registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub uuid: String,
    pub pid: u32,
    pub started_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Liveness {
    Live,
    /// The same process (pid and `startedAt`) now runs another session.
    Cleared {
        now: String,
        name: String,
    },
    Gone,
    Unknown,
}

/// `GREENBELAY_HOST_SESSIONS_DIR`, else `<claude config dir>/sessions`.
pub fn registry_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(REGISTRY_DIR_VAR).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    Some(crate::settings::config_dir()?.join("sessions"))
}

/// The live session whose `sessionId` is `uuid`.
pub fn lookup(uuid: &str) -> Option<HostSession> {
    lookup_in(&registry_dir()?, uuid)
}

/// The session registered for process `pid`.
pub fn lookup_pid(pid: u32) -> Option<HostSession> {
    lookup_pid_in(&registry_dir()?, pid)
}

/// The session this process runs under: the nearest ancestor with a registry
/// file, at most four levels up.
pub fn caller() -> Option<HostSession> {
    let sandboxed = std::env::var_os(REGISTRY_DIR_VAR).is_some_and(|dir| !dir.is_empty());
    let start = match std::env::var(CALLER_PID_VAR)
        .ok()
        .filter(|_| sandboxed)
        .and_then(|text| text.trim().parse::<u32>().ok())
    {
        Some(pid) => pid,
        None => own_parent()?,
    };
    caller_in(&registry_dir()?, start, parent_of)
}

/// [`caller`] from `start`, with `parent_of` answering for a pid's parent.
pub fn caller_in(
    dir: &Path,
    start: u32,
    parent_of: impl Fn(u32) -> Option<u32>,
) -> Option<HostSession> {
    let mut pid = start;
    for level in 0..CALLER_LEVELS {
        if let Some(found) = lookup_pid_in(dir, pid) {
            return Some(found);
        }
        if level + 1 < CALLER_LEVELS {
            pid = parent_of(pid).filter(|parent| *parent > 1)?;
        }
    }
    None
}

#[cfg(unix)]
fn own_parent() -> Option<u32> {
    Some(std::os::unix::process::parent_id())
}

#[cfg(not(unix))]
fn own_parent() -> Option<u32> {
    None
}

/// `ps -o ppid= -p <pid>`, bounded; `None` when it fails or does not answer
/// within the budget.
fn parent_of(pid: u32) -> Option<u32> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + PS_BUDGET;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill(); // past its budget; the answer is "no parent known"
                let _ = child.wait(); // reaping only
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text).ok()?;
    text.trim().parse().ok()
}

pub fn liveness(recorded: &Recorded) -> Liveness {
    match registry_dir() {
        Some(dir) => liveness_in(&dir, recorded),
        None => Liveness::Unknown,
    }
}

pub fn lookup_in(dir: &Path, uuid: &str) -> Option<HostSession> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries
        .filter_map(Result::ok)
        .filter(|entry| is_session_file(&entry.path()))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| parse(&text))
        .find(|found| found.session == uuid)
}

pub fn lookup_pid_in(dir: &Path, pid: u32) -> Option<HostSession> {
    match read_pid_file(dir, pid) {
        PidFile::Found(found) => Some(found),
        PidFile::Missing | PidFile::Unusable => None,
    }
}

/// Decided from the recorded pid's own file only: a bad file for another
/// process does not matter.
pub fn liveness_in(dir: &Path, recorded: &Recorded) -> Liveness {
    match read_pid_file(dir, recorded.pid) {
        PidFile::Unusable => Liveness::Unknown,
        PidFile::Missing => match std::fs::read_dir(dir) {
            Ok(_) => Liveness::Gone,
            Err(_) => Liveness::Unknown,
        },
        PidFile::Found(found) if found.started_at != recorded.started_at => Liveness::Gone,
        PidFile::Found(found) if found.session == recorded.uuid => Liveness::Live,
        PidFile::Found(found) => Liveness::Cleared {
            now: found.session,
            name: found.name,
        },
    }
}

enum PidFile {
    Found(HostSession),
    Missing,
    Unusable,
}

fn read_pid_file(dir: &Path, pid: u32) -> PidFile {
    match std::fs::read_to_string(dir.join(format!("{pid}.json"))) {
        Ok(text) => parse(&text).map_or(PidFile::Unusable, PidFile::Found),
        Err(error) if error.kind() == io::ErrorKind::NotFound => PidFile::Missing,
        Err(_) => PidFile::Unusable,
    }
}

fn is_session_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "json")
}

fn parse(text: &str) -> Option<HostSession> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value.get("peerProtocol")?.as_u64()? != SUPPORTED_PEER_PROTOCOL {
        return None;
    }
    let started = value.get("startedAt")?;
    let started_at = started
        .as_u64()
        // truncation intended: a fractional millisecond count is still a stable key
        .or_else(|| started.as_f64().map(|float| float as u64))?;
    Some(HostSession {
        session: value.get("sessionId")?.as_str()?.to_owned(),
        pid: u32::try_from(value.get("pid")?.as_u64()?).ok()?,
        started_at,
        name: value
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or_default()
            .to_owned(),
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    fn sandbox() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gb-host-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn register(dir: &Path, pid: u32, session: &str, started_at: u64, peer: u64) {
        let body = format!(
            r#"{{"sessionId":"{session}","name":"n-{pid}","pid":{pid},"startedAt":{started_at},"peerProtocol":{peer}}}"#
        );
        std::fs::write(dir.join(format!("{pid}.json")), body).unwrap();
    }

    fn recorded(pid: u32) -> Recorded {
        Recorded {
            uuid: "old".to_owned(),
            pid,
            started_at: 5000,
        }
    }

    #[test]
    fn the_five_states_and_a_foreign_bad_file() {
        let dir = sandbox();
        let key = dir.join("77.key");
        std::fs::write(&key, "secret").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o000)).unwrap();
        register(&dir, 1, "old", 5000, 1);
        register(&dir, 2, "new", 5000, 1);
        register(&dir, 3, "old", 9999, 1);
        std::fs::write(dir.join("5.json"), "{not json").unwrap();
        register(&dir, 6, "old", 5000, 2);
        std::fs::write(dir.join("9.json"), "{not json").unwrap();

        assert_eq!(liveness_in(&dir, &recorded(1)), Liveness::Live);
        assert_eq!(
            liveness_in(&dir, &recorded(2)),
            Liveness::Cleared {
                now: "new".to_owned(),
                name: "n-2".to_owned()
            }
        );
        assert_eq!(liveness_in(&dir, &recorded(3)), Liveness::Gone);
        assert_eq!(liveness_in(&dir, &recorded(4)), Liveness::Gone);
        assert_eq!(liveness_in(&dir, &recorded(5)), Liveness::Unknown);
        assert_eq!(liveness_in(&dir, &recorded(6)), Liveness::Unknown);
        // the unparseable 9.json belongs to another pid: pid 1 stays Live
        assert_eq!(liveness_in(&dir, &recorded(1)), Liveness::Live);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_caller_is_the_nearest_registered_ancestor_within_four_levels() {
        let dir = sandbox();
        register(&dir, 20, "far", 1, 1);
        register(&dir, 30, "near", 1, 1);
        let parents = |pid: u32| match pid {
            10 => Some(30),
            30 => Some(20),
            20 => Some(21),
            21 => Some(22),
            _ => None,
        };
        assert_eq!(caller_in(&dir, 10, parents).unwrap().session, "near");
        assert_eq!(caller_in(&dir, 21, parents), None);
        assert_eq!(caller_in(&dir, 99, parents), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unreadable_directory_is_unknown() {
        let missing = std::env::temp_dir().join("gb-host-does-not-exist-anywhere");
        assert_eq!(liveness_in(&missing, &recorded(1)), Liveness::Unknown);
        assert_eq!(lookup_in(&missing, "old"), None);
    }

    #[test]
    fn lookup_finds_by_uuid_and_by_pid_and_skips_keys_and_bad_files() {
        let dir = sandbox();
        let key = dir.join("8.key");
        std::fs::write(&key, "secret").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o000)).unwrap();
        std::fs::write(dir.join("9.json"), "garbage").unwrap();
        register(&dir, 4, "abc", 42, 1);
        let expected = HostSession {
            session: "abc".to_owned(),
            pid: 4,
            started_at: 42,
            name: "n-4".to_owned(),
        };
        assert_eq!(lookup_in(&dir, "abc"), Some(expected.clone()));
        assert_eq!(lookup_pid_in(&dir, 4), Some(expected));
        assert_eq!(lookup_in(&dir, "zzz"), None);
        assert_eq!(lookup_pid_in(&dir, 9), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
