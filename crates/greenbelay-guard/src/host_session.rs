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

const REGISTRY_DIR_VAR: &str = "GREENBELAY_HOST_SESSIONS_DIR";
const SUPPORTED_PEER_PROTOCOL: u64 = 1;

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
