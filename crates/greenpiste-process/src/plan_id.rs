//! One id per plan (`plan-id:` in the front matter), so the plan-review
//! ledger, the panel/hook baseline and `stats --time` key on the plan, not
//! on the plan-mode file, which plan mode reuses for the next plan.
//!
//! - `plan-panel` assigns the id when the front matter has none
//!   ([`ensure`]), and replaces it once the person approved that id with
//!   another body.
//! - The PostToolUse `on_plan_answered` hook records an approval
//!   ([`approve`]), before it takes the ledger lock.
//! - A file's id is *live* ([`live`]) until it was approved with another
//!   body: the hook and `record` read the id through it, so a plan edited
//!   into a new plan never inherits the old one, whether or not `plan-panel`
//!   ran in between.
//! - `plan-panel` maps each body it sees to its id under `ids/`
//!   ([`id_for_body`]), and `record` reads a row's id from that map.
//!
//! The front matter is outside `body_sha`, so writing the id never makes a
//! review stale.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use greenpiste_guard::sha256::sha256;

/// The approval record's format.
pub const APPROVAL_SCHEMA: &str = "greenpiste-plan-approval-v1";
/// The body → id map's format.
pub const ID_SCHEMA: &str = "greenpiste-plan-id-v1";

/// How long `plan-panel` waits for the ledger lock before it gives up.
const LOCK_BUDGET: Duration = Duration::from_secs(5);

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A plan's id: `p-` and 26 Crockford base32 characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlanId(String);

impl PlanId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PlanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<&str> for PlanId {
    type Error = PlanIdError;

    fn try_from(text: &str) -> Result<Self, PlanIdError> {
        let ok = text.strip_prefix("p-").is_some_and(|rest| {
            rest.len() == 26
                && rest
                    .bytes()
                    .all(|b| CROCKFORD.contains(&b.to_ascii_uppercase()))
        });
        if ok {
            Ok(PlanId(text.to_string()))
        } else {
            Err(PlanIdError::Malformed(text.to_string()))
        }
    }
}

/// Why a plan id could not be read, written or recorded. Each message says
/// what to do next.
#[derive(Debug)]
pub enum PlanIdError {
    Malformed(String),
    NoHome {
        plan: Option<PathBuf>,
    },
    Read {
        plan: PathBuf,
        source: std::io::Error,
    },
    Write {
        plan: PathBuf,
        source: std::io::Error,
    },
    Store {
        dir: PathBuf,
        plan: Option<PathBuf>,
    },
    Lock {
        source: std::io::Error,
    },
    Busy {
        plan: PathBuf,
    },
    Changed {
        plan: PathBuf,
    },
    Approval {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for PlanIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanIdError::Malformed(text) => write!(f, "{text:?} is not a plan id"),
            PlanIdError::NoHome { plan: Some(plan) } => write!(
                f,
                "no home directory, so the plan-review store cannot be located. Set HOME, then run: greenpiste plan-panel {}",
                plan.display()
            ),
            PlanIdError::NoHome { plan: None } => write!(
                f,
                "no home directory, so the plan-review store cannot be located. Set HOME"
            ),
            PlanIdError::Read { plan, source } => write!(
                f,
                "cannot read plan to add plan-id: {}: {source}",
                plan.display()
            ),
            PlanIdError::Write { plan, source } => write!(
                f,
                "cannot write plan-id: {}: {source}; make it writable, then run: greenpiste plan-panel {}",
                plan.display(),
                plan.display()
            ),
            PlanIdError::Store {
                dir,
                plan: Some(plan),
            } => write!(
                f,
                "cannot create {}; fix its permissions, then run: greenpiste plan-panel {}",
                dir.display(),
                plan.display()
            ),
            PlanIdError::Store { dir, plan: None } => {
                write!(f, "cannot create {}", dir.display())
            }
            PlanIdError::Lock { source } => write!(f, "cannot lock the ledger: {source}"),
            PlanIdError::Busy { plan } => write!(
                f,
                "ledger busy; nothing written. Run again: greenpiste plan-panel {}",
                plan.display()
            ),
            PlanIdError::Changed { plan } => write!(
                f,
                "plan changed under plan-panel; nothing written. Re-read {0}, then run: greenpiste plan-panel {0}",
                plan.display()
            ),
            PlanIdError::Approval { path, source } => {
                write!(f, "cannot write {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for PlanIdError {}

fn store() -> Option<PathBuf> {
    Some(greenpiste_guard::journal::dir()?.join("plan-review"))
}

/// The `plan-id:` of a plan's YAML front matter, when it has a valid one.
pub fn read(text: &str) -> Option<PlanId> {
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    for line in lines {
        if line.trim_end() == "---" {
            return None;
        }
        if let Some(value) = line.strip_prefix("plan-id:") {
            return PlanId::try_from(value.trim()).ok();
        }
    }
    None
}

/// `text` with `plan-id: <id>` in its front matter: the line replaced when
/// there is one, added at the end of the block otherwise, and a block made
/// when the text has none.
pub fn with_id(text: &str, id: &PlanId) -> String {
    let line = format!("plan-id: {id}");
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let close = (lines.first().map(|l| l.trim_end()) == Some("---"))
        .then(|| {
            lines
                .iter()
                .skip(1)
                .position(|l| l.trim_end() == "---")
                .map(|i| i + 1)
        })
        .flatten();
    let Some(close) = close else {
        return format!("---\n{line}\n---\n{text}");
    };
    let mut out = String::new();
    let mut replaced = false;
    for (i, l) in lines.iter().enumerate() {
        if i > 0 && i < close && l.starts_with("plan-id:") {
            out.push_str(&line);
            out.push('\n');
            replaced = true;
            continue;
        }
        if i == close && !replaced {
            out.push_str(&line);
            out.push('\n');
        }
        out.push_str(l);
    }
    out
}

/// A new id: a ULID (48-bit milliseconds, 80 random bits) as `p-` and 26
/// Crockford base32 characters. The random bits come from `/dev/urandom`;
/// off unix, or when it cannot be read, from sha256 of the time, the pid
/// and `salt`.
pub(crate) fn generate(salt: &str) -> PlanId {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let ms = now.as_millis() & ((1u128 << 48) - 1);
    let bytes = urandom().unwrap_or_else(|| {
        let seed = format!("{}:{}:{salt}", now.as_nanos(), std::process::id());
        let mut bytes = [0u8; 10];
        bytes.copy_from_slice(&sha256(seed.as_bytes())[..10]);
        bytes
    });
    let mut ulid = ms << 80;
    for (i, b) in bytes.iter().enumerate() {
        ulid |= (*b as u128) << (72 - 8 * i);
    }
    let mut out = String::from("p-");
    for i in (0..26).rev() {
        out.push(CROCKFORD[((ulid >> (5 * i)) & 31) as usize] as char);
    }
    PlanId(out)
}

#[cfg(unix)]
fn urandom() -> Option<[u8; 10]> {
    use std::io::Read;
    let mut bytes = [0u8; 10];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .ok()?;
    Some(bytes)
}

#[cfg(not(unix))]
fn urandom() -> Option<[u8; 10]> {
    None
}

/// What [`ensure`] did to the plan file, for `plan-panel`'s stderr.
#[derive(Debug, PartialEq)]
pub enum Assigned {
    /// The file already had a live id.
    Kept(PlanId),
    /// The file had none; it now has this one.
    New(PlanId),
    /// The old id was approved with another body; the file has a new one.
    Replaced { old: PlanId, new: PlanId },
}

impl Assigned {
    pub fn id(&self) -> &PlanId {
        match self {
            Assigned::Kept(id) | Assigned::New(id) => id,
            Assigned::Replaced { new, .. } => new,
        }
    }

    /// The stderr line, when the file changed.
    pub fn note(&self) -> Option<String> {
        match self {
            Assigned::Kept(_) => None,
            Assigned::New(id) => Some(format!("assigned plan-id {id}")),
            Assigned::Replaced { old, new } => {
                Some(format!("replaced plan-id {old} (approved) with {new}"))
            }
        }
    }
}

/// The body `id` was approved at, when the person approved it.
fn approved_body(store: &Path, id: &PlanId) -> Option<String> {
    let file = store.join("approved").join(format!("{id}.json"));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    (v.get("schema")?.as_str()? == APPROVAL_SCHEMA)
        .then(|| v.get("body_sha")?.as_str().map(str::to_string))
        .flatten()
}

/// The file's id when it is still live: not approved with another body.
/// A plan approved and then edited into a new plan keeps its `plan-id:`
/// line in the file, but that id is retired, so the answer is `None`.
fn live_in(store: &Path, text: &str) -> Option<PlanId> {
    let id = read(text)?;
    match approved_body(store, &id) {
        Some(body) if body != crate::plan_text::body_sha(text) => None,
        _ => Some(id),
    }
}

/// [`live_in`] over the plan-review store. With no store to ask (no home
/// directory), the file's id stands.
pub fn live(text: &str) -> Option<PlanId> {
    match store() {
        Some(store) => live_in(&store, text),
        None => read(text),
    }
}

/// Gives `plan_file` a live `plan-id:` and maps its body to it.
///
/// The file is written only when its id is absent or retired, so a second
/// run leaves it byte-identical. The write follows `plan-apply`'s protocol:
/// the ledger lock (5 s), a re-read that refuses a file changed since it was
/// read, then an atomic rename. A symlinked plan file is resolved first, so
/// the rename replaces the file, not the link. Errors name what to do next.
pub fn ensure(plan_file: &Path, text: &str) -> Result<Assigned, PlanIdError> {
    let store = store().ok_or_else(|| PlanIdError::NoHome {
        plan: Some(plan_file.to_path_buf()),
    })?;
    ensure_in(&store, plan_file, text, &|| {})
}

/// [`ensure`] over `store`; `after_lock` runs once the lock is held and
/// before the file is re-read (a test changes the file there).
pub(crate) fn ensure_in(
    store: &Path,
    plan_file: &Path,
    text: &str,
    after_lock: &dyn Fn(),
) -> Result<Assigned, PlanIdError> {
    let plan = crate::plan_review::real(plan_file);
    let body = crate::plan_text::body_sha(text);
    let salt = plan.display().to_string();
    let assigned = match read(text) {
        Some(id) => match approved_body(store, &id) {
            Some(approved) if approved != body => Assigned::Replaced {
                old: id,
                new: generate(&salt),
            },
            _ => Assigned::Kept(id),
        },
        None => Assigned::New(generate(&salt)),
    };
    if !matches!(assigned, Assigned::Kept(_)) {
        crate::plan_review::private_dir(store).ok_or_else(|| PlanIdError::Store {
            dir: store.to_path_buf(),
            plan: Some(plan.clone()),
        })?;
        let _lock =
            crate::review_outcomes::lock_within(&store.join("outcomes.jsonl.lock"), LOCK_BUDGET)
                .map_err(|source| {
                    if source.kind() == std::io::ErrorKind::TimedOut {
                        PlanIdError::Busy { plan: plan.clone() }
                    } else {
                        PlanIdError::Lock { source }
                    }
                })?;
        after_lock();
        let again = std::fs::read_to_string(&plan).map_err(|source| PlanIdError::Read {
            plan: plan.clone(),
            source,
        })?;
        if again != text {
            return Err(PlanIdError::Changed { plan });
        }
        greenpiste_guard::atomic::write_atomic(&plan, &with_id(text, assigned.id())).map_err(
            |source| PlanIdError::Write {
                plan: plan.clone(),
                source,
            },
        )?;
    }
    remember(store, &body, assigned.id());
    Ok(assigned)
}

/// Maps `body_sha` to `id` under `ids/`. A failure costs only the map
/// entry (a row then reads the file's id when the body matches), so it is
/// reported on stderr, not fatal.
fn remember(store: &Path, body_sha: &str, id: &PlanId) {
    let bin = greenpiste_guard::names::BIN;
    let dir = store.join("ids");
    if crate::plan_review::private_dir(&dir).is_none() {
        eprintln!(
            "{bin}: plan-panel: cannot create {}; rows of this plan may record no plan_id until it is writable",
            dir.display()
        );
        return;
    }
    let record =
        serde_json::json!({"schema": ID_SCHEMA, "body_sha": body_sha, "plan_id": id.as_str()})
            .to_string();
    let file = dir.join(format!("{body_sha}.json"));
    if let Err(e) = greenpiste_guard::atomic::write_atomic(&file, &record) {
        eprintln!(
            "{bin}: plan-panel: cannot record the plan-id of body {}: {}: {e}",
            &body_sha[..12.min(body_sha.len())],
            file.display()
        );
    }
}

/// The id `plan-panel` mapped `body_sha` to.
pub fn id_for_body(body_sha: &str) -> Option<PlanId> {
    let file = store()?.join("ids").join(format!("{body_sha}.json"));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    if v.get("schema")?.as_str()? != ID_SCHEMA {
        return None;
    }
    PlanId::try_from(v.get("plan_id")?.as_str()?).ok()
}

/// Records that the person approved `id` at `body_sha`: one file, written
/// atomically and without the ledger lock, so a busy ledger never loses it.
pub fn approve(id: &PlanId, body_sha: &str) -> Result<(), PlanIdError> {
    let store = store().ok_or(PlanIdError::NoHome { plan: None })?;
    let dir = store.join("approved");
    crate::plan_review::private_dir(&dir).ok_or_else(|| PlanIdError::Store {
        dir: dir.clone(),
        plan: None,
    })?;
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let record = serde_json::json!({
        "schema": APPROVAL_SCHEMA, "plan_id": id.as_str(), "body_sha": body_sha, "approved_at": at,
    })
    .to_string();
    let path = dir.join(format!("{id}.json"));
    greenpiste_guard::atomic::write_atomic(&path, &record)
        .map_err(|source| PlanIdError::Approval { path, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh empty directory under the system temp dir.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "greenpiste-plan-id-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir); // absent on a first run
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn generate_is_valid_and_distinct() {
        let a = generate("x");
        let b = generate("x");
        assert!(PlanId::try_from(a.as_str()).is_ok(), "{a}");
        assert_ne!(a, b);
        assert!(PlanId::try_from("p-short").is_err());
        assert!(PlanId::try_from("x-01M7FVXGP6XFZ6EEK02527AQE2").is_err());
    }

    #[test]
    fn read_and_write_the_front_matter() {
        assert_eq!(read("# T\n"), None);
        let id = generate("t");
        let made = with_id("# T\nbody\n", &id);
        assert_eq!(made, format!("---\nplan-id: {id}\n---\n# T\nbody\n"));
        assert_eq!(read(&made), Some(id.clone()));
        let fm = "---\nstatus: active\nbranch: feat/x\n---\n# T\n";
        let added = with_id(fm, &id);
        assert_eq!(
            added,
            format!("---\nstatus: active\nbranch: feat/x\nplan-id: {id}\n---\n# T\n")
        );
        let other = generate("u");
        let replaced = with_id(&added, &other);
        assert_eq!(read(&replaced), Some(other));
        assert_eq!(replaced.matches("plan-id:").count(), 1);
        // A `plan-id:` after the front matter is body text, not the id.
        assert_eq!(read("---\nstatus: x\n---\nplan-id: p-0000\n"), None);
    }

    #[test]
    fn the_id_is_outside_the_body_sha() {
        let text = "---\nstatus: active\n---\n# T\n\n## Goal\nG.\n";
        let with = with_id(text, &generate("s"));
        assert_eq!(
            crate::plan_text::body_sha(text),
            crate::plan_text::body_sha(&with)
        );
    }

    #[test]
    fn a_file_changed_between_the_read_and_the_lock_is_refused_untouched() {
        let dir = scratch("changed");
        let store = dir.join("plan-review");
        let plan = dir.join("plan.md");
        let text = "# T\n\n## Goal\nG.\n";
        std::fs::write(&plan, text).unwrap();
        // The agent's own Edit lands once plan-panel holds the lock.
        let edited = format!("{text}\nEdited meanwhile.\n");
        let err = ensure_in(&store, &plan, text, &|| {
            std::fs::write(&plan, &edited).unwrap();
        })
        .unwrap_err();
        assert!(matches!(err, PlanIdError::Changed { .. }), "{err}");
        let said = err.to_string();
        assert!(
            said.starts_with("plan changed under plan-panel; nothing written. Re-read "),
            "{said}"
        );
        assert!(said.contains("then run: greenpiste plan-panel "), "{said}");
        assert_eq!(std::fs::read_to_string(&plan).unwrap(), edited);
    }

    #[test]
    fn a_retired_id_is_not_live_and_an_untouched_one_is() {
        let dir = scratch("live");
        let store = dir.join("plan-review");
        let id = generate("l");
        let a = with_id("# A\n\n## Goal\nA.\n", &id);
        let b = with_id("# B\n\n## Goal\nB, another plan.\n", &id);
        assert_eq!(live_in(&store, &a), Some(id.clone()), "no approval yet");
        std::fs::create_dir_all(store.join("approved")).unwrap();
        let record = serde_json::json!({
            "schema": APPROVAL_SCHEMA, "plan_id": id.as_str(),
            "body_sha": crate::plan_text::body_sha(&a), "approved_at": 1,
        });
        std::fs::write(
            store.join("approved").join(format!("{id}.json")),
            record.to_string(),
        )
        .unwrap();
        assert_eq!(live_in(&store, &a), Some(id), "the approved body itself");
        assert_eq!(live_in(&store, &b), None, "an edit into another plan");
    }
}
