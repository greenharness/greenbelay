//! Owner records: which session holds a worktree or a programme ticket.
//!
//! One JSON file per slot under `<data dir>/owners/`. A free slot is taken
//! by writing a temp file in full and `link`ing it into place, so a reader
//! never sees an empty record and a losing racer gets `EEXIST`. Every change
//! to an existing record holds `<record>.lock` (an `flock`, which the kernel
//! releases when the holder dies) and re-reads the record under it. The lock
//! file is never renamed or removed. Local filesystems only.

use crate::host_session::{self, Liveness, Recorded};
use serde_json::{json, Map, Value};
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const RECORD_VERSION: u64 = 1;
const STALE_AFTER_SECONDS: u64 = 8 * 60 * 60;
const REFRESH_EVERY_SECONDS: u64 = 60;
const LOCK_BUDGET: Duration = Duration::from_millis(500);
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Where owner records live and which host registry judges their liveness.
#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
    pub registry: PathBuf,
}

impl Store {
    /// The store a hook or command uses: `<data dir>/owners`, and the host
    /// registry directory. `None` when either cannot be found.
    pub fn ambient() -> Option<Store> {
        Some(Store {
            root: crate::settings::data_dir()?.join("owners"),
            registry: host_session::registry_dir()?,
        })
    }
}

#[derive(Debug)]
pub enum Error {
    InvalidSegment {
        segment: String,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidSegment { segment } => {
                write!(
                    out,
                    "invalid ticket key segment {segment:?}: use [A-Za-z0-9._-]+, not . or .."
                )
            }
            Error::Io {
                operation,
                path,
                source,
            } => write!(out, "{operation} {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

fn io_error<'a>(operation: &'static str, path: &'a Path) -> impl FnOnce(io::Error) -> Error + 'a {
    move |source| Error::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

/// A session as an owner record keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub session: String,
    pub pid: u32,
    pub started_at: u64,
    pub name: String,
    pub reference: Option<String>,
}

impl Identity {
    fn recorded(&self) -> Recorded {
        Recorded {
            uuid: self.session.clone(),
            pid: self.pid,
            started_at: self.started_at,
        }
    }

    fn same_process(&self, other: &Identity) -> bool {
        self.pid == other.pid && self.started_at == other.started_at
    }
}

/// What a record is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Worktree(PathBuf),
    Ticket { map_dir: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub place: Place,
    pub repo_identity: PathBuf,
    pub branch: String,
    pub identity: Identity,
    pub claimed_at: u64,
    pub refreshed_at: u64,
}

/// The claimant's side of a claim; the place and the times come from the call.
#[derive(Clone, Debug)]
pub struct Claim {
    pub repo_identity: PathBuf,
    pub branch: String,
    pub identity: Identity,
}

/// A validated `repo/programme/ticket` key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TicketKey {
    repo: String,
    programme: String,
    ticket: String,
}

impl TicketKey {
    pub fn new(repo: &str, programme: &str, ticket: &str) -> Result<TicketKey, Error> {
        for segment in [repo, programme, ticket] {
            if !valid_segment(segment) {
                return Err(Error::InvalidSegment {
                    segment: segment.to_owned(),
                });
            }
        }
        Ok(TicketKey {
            repo: repo.to_owned(),
            programme: programme.to_owned(),
            ticket: ticket.to_owned(),
        })
    }
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A record's address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    Worktree(PathBuf),
    Ticket(TicketKey),
}

impl Slot {
    pub fn record_path(&self, store: &Store) -> PathBuf {
        match self {
            Slot::Worktree(path) => {
                let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
                store.root.join("worktree").join(format!(
                    "{:016x}.json",
                    fnv1a64(real.as_os_str().as_encoded_bytes())
                ))
            }
            Slot::Ticket(key) => store
                .root
                .join("ticket")
                .join(&key.repo)
                .join(&key.programme)
                .join(format!("{}.json", key.ticket)),
        }
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

/// How a record stands, decided from its owner's own pid file and its age.
/// Precedence: `Gone`, then `Stale`, then the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Live,
    Cleared { now: String, name: String },
    Gone,
    Stale,
    Unknown,
}

pub fn state_of(store: &Store, owner: &Owner, now: u64) -> State {
    let liveness = host_session::liveness_in(&store.registry, &owner.identity.recorded());
    if liveness == Liveness::Gone {
        return State::Gone;
    }
    if now.saturating_sub(owner.refreshed_at) >= STALE_AFTER_SECONDS {
        return State::Stale;
    }
    match liveness {
        Liveness::Live => State::Live,
        Liveness::Cleared { now, name } => State::Cleared { now, name },
        Liveness::Unknown | Liveness::Gone => State::Unknown,
    }
}

impl State {
    /// The one word `--json` carries.
    pub fn word(&self) -> &'static str {
        match self {
            State::Live => "live",
            State::Cleared { .. } => "cleared",
            State::Gone => "gone",
            State::Stale => "stale",
            State::Unknown => "unknown",
        }
    }

    /// The state as a person reads it, state word first; the board and
    /// `owner show` print the same text.
    pub fn describe(&self) -> String {
        match self {
            State::Live => "live, message it".to_owned(),
            State::Cleared { now, name } => {
                format!("cleared since: {name} now runs {now}, which has none of this work")
            }
            State::Gone => "gone".to_owned(),
            State::Stale => "stale, not refreshed for 8 hours".to_owned(),
            State::Unknown => "unknown: host registry unreadable".to_owned(),
        }
    }
}

impl Owner {
    /// `name [ref]`, or the name alone.
    pub fn holder(&self) -> String {
        match &self.identity.reference {
            Some(reference) => format!("{} [{reference}]", self.identity.name),
            None => self.identity.name.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contest {
    Respect,
    Steal,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Claimed,
    Refreshed,
    TakenOver { from: Box<Identity> },
    HeldBy { owner: Box<Owner>, state: State },
    LockBusy,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Released {
    Released,
    AlreadyFree,
    HeldBy { owner: Box<Owner>, state: State },
    LockBusy,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reading {
    Free,
    Held {
        owner: Box<Owner>,
        state: State,
    },
    /// The record exists but is not readable as a record.
    Unknown,
}

enum Record {
    Absent,
    Present(Box<Owner>),
    Unreadable,
}

pub fn claim_worktree(
    store: &Store,
    worktree: &Path,
    claim: &Claim,
    contest: Contest,
    now: u64,
) -> Result<Outcome, Error> {
    let record = Slot::Worktree(worktree.to_path_buf()).record_path(store);
    let real = std::fs::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    claim_at(
        &record,
        fresh(Place::Worktree(real), claim, now),
        store,
        contest,
        now,
    )
}

pub fn claim_ticket(
    store: &Store,
    key: &TicketKey,
    map_dir: &Path,
    claim: &Claim,
    contest: Contest,
    now: u64,
) -> Result<Outcome, Error> {
    let record = Slot::Ticket(key.clone()).record_path(store);
    let place = Place::Ticket {
        map_dir: map_dir.to_path_buf(),
    };
    claim_at(&record, fresh(place, claim, now), store, contest, now)
}

fn fresh(place: Place, claim: &Claim, now: u64) -> Owner {
    Owner {
        place,
        repo_identity: claim.repo_identity.clone(),
        branch: claim.branch.clone(),
        identity: claim.identity.clone(),
        claimed_at: now,
        refreshed_at: now,
    }
}

enum Verdict {
    Refresh,
    TakeOver,
    Reclaim,
    Held(State),
}

fn decide(store: &Store, existing: &Owner, fresh: &Owner, contest: Contest, now: u64) -> Verdict {
    if existing.identity.session == fresh.identity.session {
        return Verdict::Refresh;
    }
    if existing.identity.same_process(&fresh.identity) {
        return Verdict::TakeOver;
    }
    match state_of(store, existing, now) {
        State::Gone | State::Stale | State::Cleared { .. } => Verdict::Reclaim,
        held @ (State::Live | State::Unknown) => match contest {
            Contest::Steal => Verdict::TakeOver,
            Contest::Respect => Verdict::Held(held),
        },
    }
}

const CLAIM_ATTEMPTS: u32 = 3;

fn claim_at(
    record: &Path,
    fresh: Owner,
    store: &Store,
    contest: Contest,
    now: u64,
) -> Result<Outcome, Error> {
    for _ in 0..CLAIM_ATTEMPTS {
        match read_record(record) {
            Record::Absent => {
                if create(record, &fresh)? {
                    return Ok(Outcome::Claimed);
                }
                continue;
            }
            Record::Present(existing) => {
                if let Verdict::Held(state) = decide(store, &existing, &fresh, contest, now) {
                    return Ok(Outcome::HeldBy {
                        owner: existing,
                        state,
                    });
                }
            }
            Record::Unreadable => {}
        }
        let Some(_guard) = lock_record(record)? else {
            return Ok(Outcome::LockBusy);
        };
        match read_record(record) {
            Record::Absent => {
                if create(record, &fresh)? {
                    return Ok(Outcome::Claimed);
                }
            }
            Record::Unreadable => {
                replace(record, &fresh)?;
                return Ok(Outcome::Claimed);
            }
            Record::Present(existing) => {
                return match decide(store, &existing, &fresh, contest, now) {
                    Verdict::Held(state) => Ok(Outcome::HeldBy {
                        owner: existing,
                        state,
                    }),
                    Verdict::Refresh => {
                        let kept = Owner {
                            claimed_at: existing.claimed_at,
                            ..fresh
                        };
                        replace(record, &kept)?;
                        Ok(Outcome::Refreshed)
                    }
                    Verdict::TakeOver => {
                        replace(record, &fresh)?;
                        Ok(Outcome::TakenOver {
                            from: Box::new(existing.identity),
                        })
                    }
                    Verdict::Reclaim => {
                        replace(record, &fresh)?;
                        Ok(Outcome::Claimed)
                    }
                };
            }
        }
    }
    Ok(Outcome::LockBusy)
}

/// Touch `refreshed_at` of the caller's own record, writing at most once per
/// 60 s. `None` when the slot has no record.
pub fn refresh(
    store: &Store,
    slot: &Slot,
    session: &str,
    now: u64,
) -> Result<Option<Outcome>, Error> {
    let record = slot.record_path(store);
    let Record::Present(seen) = read_record(&record) else {
        return Ok(None);
    };
    if seen.identity.session != session {
        let state = state_of(store, &seen, now);
        return Ok(Some(Outcome::HeldBy { owner: seen, state }));
    }
    if now.saturating_sub(seen.refreshed_at) < REFRESH_EVERY_SECONDS {
        return Ok(Some(Outcome::Refreshed));
    }
    let Some(_guard) = lock_record(&record)? else {
        return Ok(Some(Outcome::LockBusy));
    };
    match read_record(&record) {
        Record::Present(current) if current.identity.session == session => {
            let touched = Owner {
                refreshed_at: now,
                ..*current
            };
            replace(&record, &touched)?;
            Ok(Some(Outcome::Refreshed))
        }
        Record::Present(current) => {
            let state = state_of(store, &current, now);
            Ok(Some(Outcome::HeldBy {
                owner: current,
                state,
            }))
        }
        Record::Absent | Record::Unreadable => Ok(None),
    }
}

/// Free the slot, when `session` holds it.
pub fn release(store: &Store, slot: &Slot, session: &str, now: u64) -> Result<Released, Error> {
    let record = slot.record_path(store);
    if matches!(read_record(&record), Record::Absent) {
        return Ok(Released::AlreadyFree);
    }
    let Some(_guard) = lock_record(&record)? else {
        return Ok(Released::LockBusy);
    };
    match read_record(&record) {
        Record::Absent => Ok(Released::AlreadyFree),
        Record::Present(current) if current.identity.session != session => {
            let state = state_of(store, &current, now);
            Ok(Released::HeldBy {
                owner: current,
                state,
            })
        }
        Record::Present(_) | Record::Unreadable => {
            match std::fs::remove_file(&record) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error("remove", &record)(error)),
            }
            Ok(Released::Released)
        }
    }
}

pub fn read_worktree(store: &Store, worktree: &Path, now: u64) -> Reading {
    read_slot(store, &Slot::Worktree(worktree.to_path_buf()), now)
}

pub fn read_ticket(store: &Store, key: &TicketKey, now: u64) -> Reading {
    read_slot(store, &Slot::Ticket(key.clone()), now)
}

fn read_slot(store: &Store, slot: &Slot, now: u64) -> Reading {
    match read_record(&slot.record_path(store)) {
        Record::Absent => Reading::Free,
        Record::Unreadable => Reading::Unknown,
        Record::Present(owner) => {
            let state = state_of(store, &owner, now);
            Reading::Held { owner, state }
        }
    }
}

/// An unparseable record is read once more before it is called unreadable:
/// a reader should not lose to a writer in the middle of a rename.
fn read_record(path: &Path) -> Record {
    match read_record_once(path) {
        Record::Unreadable => read_record_once(path),
        settled => settled,
    }
}

fn read_record_once(path: &Path) -> Record {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            parse(&text).map_or(Record::Unreadable, |owner| Record::Present(Box::new(owner)))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Record::Absent,
        Err(_) => Record::Unreadable,
    }
}

fn lock_record(record: &Path) -> Result<Option<std::fs::File>, Error> {
    let mut name = record.as_os_str().to_os_string();
    name.push(".lock");
    let lock_path = PathBuf::from(name);
    ensure_parent(&lock_path)?;
    crate::lock::lock_within(&lock_path, LOCK_BUDGET).map_err(io_error("lock", &lock_path))
}

fn ensure_parent(path: &Path) -> Result<(), Error> {
    match path.parent() {
        Some(parent) => std::fs::create_dir_all(parent).map_err(io_error("create", parent)),
        None => Ok(()),
    }
}

/// Write `owner` to a fsynced same-directory temp file.
fn write_temp(record: &Path, owner: &Owner) -> Result<PathBuf, Error> {
    ensure_parent(record)?;
    let dir = record.parent().unwrap_or_else(|| Path::new("."));
    let temp = dir.join(format!(
        "{}owner-{}-{}.tmp",
        crate::names::TEMP_PREFIX,
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let body = serde_json::to_string(&to_json(owner)).map_err(|source| Error::Io {
        operation: "encode",
        path: record.to_path_buf(),
        source: io::Error::other(source),
    })?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&temp).and_then(|mut file| {
        file.write_all(body.as_bytes())?;
        file.sync_all()
    });
    match written {
        Ok(()) => Ok(temp),
        Err(error) => {
            let _ = std::fs::remove_file(&temp); // litter only; the write error is the one reported
            Err(io_error("write", &temp)(error))
        }
    }
}

/// Take a free slot. `false` when another claimant got there first.
fn create(record: &Path, owner: &Owner) -> Result<bool, Error> {
    let temp = write_temp(record, owner)?;
    let linked = std::fs::hard_link(&temp, record);
    let _ = std::fs::remove_file(&temp); // litter only; the record is already in place or the link error is reported
    match linked {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(io_error("link", record)(error)),
    }
}

/// Replace an existing record: temp, fsync, rename.
fn replace(record: &Path, owner: &Owner) -> Result<(), Error> {
    let temp = write_temp(record, owner)?;
    std::fs::rename(&temp, record).map_err(|error| {
        let _ = std::fs::remove_file(&temp); // litter only; the rename error is the one reported
        io_error("rename", record)(error)
    })
}

fn to_json(owner: &Owner) -> Value {
    let mut identity = Map::new();
    identity.insert("session".into(), json!(owner.identity.session));
    identity.insert("pid".into(), json!(owner.identity.pid));
    identity.insert("started_at".into(), json!(owner.identity.started_at));
    identity.insert("name".into(), json!(owner.identity.name));
    if let Some(reference) = &owner.identity.reference {
        identity.insert("ref".into(), json!(reference));
    }
    let mut record = Map::new();
    record.insert("v".into(), json!(RECORD_VERSION));
    match &owner.place {
        Place::Worktree(path) => record.insert("worktree".into(), json!(path.to_string_lossy())),
        Place::Ticket { map_dir } => {
            record.insert("map_dir".into(), json!(map_dir.to_string_lossy()))
        }
    };
    record.insert(
        "repo_identity".into(),
        json!(owner.repo_identity.to_string_lossy()),
    );
    record.insert("branch".into(), json!(owner.branch));
    record.insert("identity".into(), Value::Object(identity));
    record.insert("claimed_at".into(), json!(owner.claimed_at));
    record.insert("refreshed_at".into(), json!(owner.refreshed_at));
    Value::Object(record)
}

fn parse(text: &str) -> Option<Owner> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("v")?.as_u64()? != RECORD_VERSION {
        return None;
    }
    let text_of = |field: &Value, key: &str| field.get(key)?.as_str().map(str::to_owned);
    let place = match (value.get("worktree"), value.get("map_dir")) {
        (Some(path), None) => Place::Worktree(PathBuf::from(path.as_str()?)),
        (None, Some(path)) => Place::Ticket {
            map_dir: PathBuf::from(path.as_str()?),
        },
        _ => return None,
    };
    let who = value.get("identity")?;
    Some(Owner {
        place,
        repo_identity: PathBuf::from(text_of(&value, "repo_identity")?),
        branch: text_of(&value, "branch")?,
        identity: Identity {
            session: text_of(who, "session")?,
            pid: u32::try_from(who.get("pid")?.as_u64()?).ok()?,
            started_at: who.get("started_at")?.as_u64()?,
            name: text_of(who, "name")?,
            reference: text_of(who, "ref"),
        },
        claimed_at: value.get("claimed_at")?.as_u64()?,
        refreshed_at: value.get("refreshed_at")?.as_u64()?,
    })
}

/// `greenbelay owner claim|release|show`.
#[path = "owner/cli.rs"]
pub mod command;

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000;

    struct Sandbox {
        base: PathBuf,
        store: Store,
        worktree: PathBuf,
    }

    impl Sandbox {
        fn new() -> Sandbox {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let base = std::env::temp_dir().join(format!(
                "gb-owner-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let registry = base.join("sessions");
            let worktree = base.join("wt");
            std::fs::create_dir_all(&registry).unwrap();
            std::fs::create_dir_all(&worktree).unwrap();
            Sandbox {
                store: Store {
                    root: base.join("owners"),
                    registry,
                },
                worktree,
                base,
            }
        }

        fn register(&self, who: &Identity) {
            let body = format!(
                r#"{{"sessionId":"{}","name":"{}","pid":{},"startedAt":{},"peerProtocol":1}}"#,
                who.session, who.name, who.pid, who.started_at
            );
            std::fs::write(self.store.registry.join(format!("{}.json", who.pid)), body).unwrap();
        }

        fn claim(&self, who: &Identity, contest: Contest, now: u64) -> Outcome {
            claim_worktree(&self.store, &self.worktree, &claim_of(who), contest, now).unwrap()
        }

        fn slot(&self) -> Slot {
            Slot::Worktree(self.worktree.clone())
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base); // best-effort cleanup of a scratch dir
        }
    }

    fn who(session: &str, pid: u32) -> Identity {
        Identity {
            session: session.to_owned(),
            pid,
            started_at: 7,
            name: format!("name-{pid}"),
            reference: None,
        }
    }

    fn claim_of(identity: &Identity) -> Claim {
        Claim {
            repo_identity: PathBuf::from("/repo/.git"),
            branch: "feat/x".to_owned(),
            identity: identity.clone(),
        }
    }

    fn held_state(sandbox: &Sandbox) -> State {
        match read_worktree(&sandbox.store, &sandbox.worktree, NOW) {
            Reading::Held { state, .. } => state,
            other => panic!("expected a held record, got {other:?}"),
        }
    }

    #[test]
    fn fnv1a64_matches_the_published_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn a_record_round_trips_and_lives_at_the_hashed_path() {
        let sandbox = Sandbox::new();
        let mut owner = who("s1", 10);
        owner.reference = Some("417dc6".to_owned());
        sandbox.register(&owner);
        assert_eq!(
            sandbox.claim(&owner, Contest::Respect, NOW),
            Outcome::Claimed
        );
        let real = std::fs::canonicalize(&sandbox.worktree).unwrap();
        let path = sandbox.store.root.join("worktree").join(format!(
            "{:016x}.json",
            fnv1a64(real.as_os_str().as_encoded_bytes())
        ));
        let text = std::fs::read_to_string(path).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["identity"]["ref"], "417dc6");
        match read_worktree(&sandbox.store, &sandbox.worktree, NOW) {
            Reading::Held { owner: held, state } => {
                assert_eq!(held.identity, owner);
                assert_eq!(held.place, Place::Worktree(real));
                assert_eq!(state, State::Live);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn precedence_is_gone_then_stale_then_live_or_unknown() {
        let sandbox = Sandbox::new();
        let owner = who("s1", 10);
        sandbox.register(&owner);
        sandbox.claim(&owner, Contest::Respect, NOW);
        assert_eq!(held_state(&sandbox), State::Live);

        let nine_hours = 9 * 60 * 60;
        let stored = match read_record(&sandbox.slot().record_path(&sandbox.store)) {
            Record::Present(owner) => *owner,
            _ => panic!("no record"),
        };
        assert_eq!(
            state_of(&sandbox.store, &stored, NOW + nine_hours),
            State::Stale
        );

        std::fs::remove_file(sandbox.store.registry.join("10.json")).unwrap();
        assert_eq!(
            state_of(&sandbox.store, &stored, NOW + nine_hours),
            State::Gone
        );
        assert_eq!(state_of(&sandbox.store, &stored, NOW), State::Gone);

        std::fs::write(sandbox.store.registry.join("10.json"), "{bad").unwrap();
        assert_eq!(state_of(&sandbox.store, &stored, NOW), State::Unknown);
    }

    #[test]
    fn a_live_owner_holds_unless_stolen_and_a_stale_one_is_free() {
        let sandbox = Sandbox::new();
        let first = who("s1", 10);
        let second = who("s2", 20);
        sandbox.register(&first);
        sandbox.register(&second);
        sandbox.claim(&first, Contest::Respect, NOW);
        match sandbox.claim(&second, Contest::Respect, NOW + 5) {
            Outcome::HeldBy { owner, state } => {
                assert_eq!(owner.identity.session, "s1");
                assert_eq!(state, State::Live);
            }
            other => panic!("{other:?}"),
        }
        match sandbox.claim(&second, Contest::Steal, NOW + 5) {
            Outcome::TakenOver { from } => assert_eq!(from.session, "s1"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            sandbox.claim(&first, Contest::Respect, NOW + 9 * 60 * 60),
            Outcome::Claimed
        );
    }

    #[test]
    fn a_cleared_record_is_free_for_others_and_taken_over_by_its_successor() {
        let sandbox = Sandbox::new();
        let before = who("old", 10);
        sandbox.register(&before);
        sandbox.claim(&before, Contest::Respect, NOW);
        let after = who("new", 10);
        sandbox.register(&after);
        assert!(matches!(held_state(&sandbox), State::Cleared { .. }));

        let stranger = who("other", 30);
        sandbox.register(&stranger);
        let successor = sandbox.claim(&after, Contest::Respect, NOW + 1);
        match successor {
            Outcome::TakenOver { from } => assert_eq!(from.session, "old"),
            other => panic!("{other:?}"),
        }

        // a second cleared record: a stranger reclaims it
        let sandbox = Sandbox::new();
        sandbox.register(&before);
        sandbox.claim(&before, Contest::Respect, NOW);
        sandbox.register(&after);
        sandbox.register(&stranger);
        assert_eq!(
            sandbox.claim(&stranger, Contest::Respect, NOW + 1),
            Outcome::Claimed
        );
    }

    fn race(sandbox: &Sandbox, racers: &[Identity]) -> usize {
        let barrier = std::sync::Barrier::new(racers.len());
        let outcomes: Vec<Outcome> = std::thread::scope(|scope| {
            let handles: Vec<_> = racers
                .iter()
                .map(|racer| {
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        sandbox.claim(racer, Contest::Respect, NOW + 1)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        outcomes
            .iter()
            .filter(|outcome| **outcome == Outcome::Claimed)
            .count()
    }

    #[test]
    fn concurrent_claims_on_an_empty_slot_have_one_winner() {
        for _ in 0..20 {
            let sandbox = Sandbox::new();
            let racers = [who("a", 11), who("b", 12), who("c", 13)];
            racers.iter().for_each(|racer| sandbox.register(racer));
            assert_eq!(race(&sandbox, &racers), 1);
        }
    }

    #[test]
    fn concurrent_claims_on_a_gone_record_have_one_winner() {
        for _ in 0..20 {
            let sandbox = Sandbox::new();
            let gone = who("gone", 99);
            sandbox.claim(&gone, Contest::Respect, NOW);
            let racers = [who("a", 11), who("b", 12), who("c", 13)];
            racers.iter().for_each(|racer| sandbox.register(racer));
            assert_eq!(race(&sandbox, &racers), 1);
        }
    }

    extern "C" {
        fn fork() -> i32;
        fn kill(pid: i32, signal: i32) -> i32;
        fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
        fn sleep(seconds: u32) -> u32;
    }

    #[test]
    fn a_claim_after_the_lock_holder_dies_succeeds() {
        let sandbox = Sandbox::new();
        let gone = who("gone", 99);
        sandbox.claim(&gone, Contest::Respect, NOW);
        let claimant = who("a", 11);
        sandbox.register(&claimant);

        let record = sandbox.slot().record_path(&sandbox.store);
        let mut lock_name = record.as_os_str().to_os_string();
        lock_name.push(".lock");
        let held = crate::lock::lock(Path::new(&lock_name)).unwrap();
        // SAFETY: the child only sleeps in a loop and is killed below; the
        // lock lives on the open file description it inherits.
        let child = unsafe { fork() };
        assert!(child >= 0);
        if child == 0 {
            loop {
                // SAFETY: `sleep` takes no pointers.
                unsafe { sleep(1) };
            }
        }
        drop(held);

        assert_eq!(
            sandbox.claim(&claimant, Contest::Respect, NOW + 1),
            Outcome::LockBusy
        );

        // SAFETY: `child` is the pid just forked; SIGKILL then reap it.
        unsafe {
            kill(child, 9);
            waitpid(child, std::ptr::null_mut(), 0);
        }
        assert_eq!(
            sandbox.claim(&claimant, Contest::Respect, NOW + 2),
            Outcome::Claimed
        );
    }

    #[test]
    fn refresh_writes_at_most_once_per_minute_and_release_frees_the_slot() {
        let sandbox = Sandbox::new();
        let owner = who("s1", 10);
        sandbox.register(&owner);
        sandbox.claim(&owner, Contest::Respect, NOW);
        let slot = sandbox.slot();
        let refreshed_at = |sandbox: &Sandbox| match read_record(&slot.record_path(&sandbox.store))
        {
            Record::Present(record) => record.refreshed_at,
            _ => panic!("no record"),
        };
        refresh(&sandbox.store, &slot, "s1", NOW + 30).unwrap();
        assert_eq!(refreshed_at(&sandbox), NOW);
        let outcome = refresh(&sandbox.store, &slot, "s1", NOW + 61).unwrap();
        assert_eq!(outcome, Some(Outcome::Refreshed));
        assert_eq!(refreshed_at(&sandbox), NOW + 61);

        assert!(matches!(
            release(&sandbox.store, &slot, "someone-else", NOW + 62).unwrap(),
            Released::HeldBy { .. }
        ));
        assert_eq!(
            release(&sandbox.store, &slot, "s1", NOW + 62).unwrap(),
            Released::Released
        );
        assert_eq!(
            release(&sandbox.store, &slot, "s1", NOW + 63).unwrap(),
            Released::AlreadyFree
        );
        assert_eq!(
            read_worktree(&sandbox.store, &sandbox.worktree, NOW),
            Reading::Free
        );
    }

    #[test]
    fn an_unparseable_record_reads_as_unknown() {
        let sandbox = Sandbox::new();
        let record = sandbox.slot().record_path(&sandbox.store);
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, "{").unwrap();
        assert_eq!(
            read_worktree(&sandbox.store, &sandbox.worktree, NOW),
            Reading::Unknown
        );
    }

    #[test]
    fn ticket_records_live_under_repo_programme_ticket_and_refuse_odd_segments() {
        let sandbox = Sandbox::new();
        let key = TicketKey::new("decisions", "follow-ups", "f107.theme_x-1").unwrap();
        let owner = who("s1", 10);
        sandbox.register(&owner);
        let outcome = claim_ticket(
            &sandbox.store,
            &key,
            Path::new("/maps/follow-ups"),
            &claim_of(&owner),
            Contest::Respect,
            NOW,
        )
        .unwrap();
        assert_eq!(outcome, Outcome::Claimed);
        assert!(sandbox
            .store
            .root
            .join("ticket/decisions/follow-ups/f107.theme_x-1.json")
            .is_file());
        match read_ticket(&sandbox.store, &key, NOW) {
            Reading::Held { owner: held, .. } => assert_eq!(
                held.place,
                Place::Ticket {
                    map_dir: PathBuf::from("/maps/follow-ups")
                }
            ),
            other => panic!("{other:?}"),
        }
        for bad in ["", ".", "..", "a/b", "a b", "../x", "é"] {
            assert!(TicketKey::new(bad, "p", "t").is_err(), "{bad:?}");
            assert!(TicketKey::new("r", bad, "t").is_err(), "{bad:?}");
            assert!(TicketKey::new("r", "p", bad).is_err(), "{bad:?}");
        }
    }
}
