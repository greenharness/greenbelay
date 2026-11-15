//! One id per plan (`plan-id:` in the front matter), so the plan-review
//! ledger, the panel/hook baseline and `stats --time` key on the plan, not
//! on the plan-mode file, which plan mode reuses for the next plan.
//!
//! - `plan-panel` assigns the id when the front matter has none
//!   ([`ensure`]), and replaces it once the person approved that id with
//!   another body.
//! - The PostToolUse `on_plan_answered` hook records an approval
//!   ([`approve`]), before it takes the ledger lock.
//! - `plan-panel` maps each body it sees to its id under `ids/`
//!   ([`id_for_body`]), and `record` reads a row's id from that map.
//!
//! The front matter is outside `body_sha`, so writing the id never makes a
//! review stale.

use std::path::{Path, PathBuf};

use greenpiste_guard::sha256::sha256;

/// The approval record's format.
pub const APPROVAL_SCHEMA: &str = "greenpiste-plan-approval-v1";
/// The body → id map's format.
pub const ID_SCHEMA: &str = "greenpiste-plan-id-v1";

fn store() -> Option<PathBuf> {
    Some(greenpiste_guard::journal::dir()?.join("plan-review"))
}

/// The `plan-id:` of a plan's YAML front matter, when it has one.
pub fn read(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    for l in lines {
        if l.trim_end() == "---" {
            return None;
        }
        if let Some(v) = l.strip_prefix("plan-id:") {
            let v = v.trim();
            return valid(v).then(|| v.to_string());
        }
    }
    None
}

/// `p-` and 26 Crockford base32 characters.
fn valid(id: &str) -> bool {
    id.strip_prefix("p-").is_some_and(|r| {
        r.len() == 26
            && r.bytes()
                .all(|b| CROCKFORD.contains(&b.to_ascii_uppercase()))
    })
}

/// `text` with `plan-id: <id>` in its front matter: the line replaced when
/// there is one, added at the end of the block otherwise, and a block made
/// when the text has none.
pub fn with_id(text: &str, id: &str) -> String {
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

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A new id: a ULID (48-bit milliseconds, 80 random bits) as `p-` and 26
/// Crockford base32 characters. The random bits come from `/dev/urandom`;
/// off unix, or when it cannot be read, from sha256 of the time, the pid
/// and `salt`.
pub fn generate(salt: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let ms = now.as_millis() & ((1u128 << 48) - 1);
    let mut bytes = [0u8; 10];
    if !urandom(&mut bytes) {
        let seed = format!("{}:{}:{salt}", now.as_nanos(), std::process::id());
        bytes.copy_from_slice(&sha256(seed.as_bytes())[..10]);
    }
    let mut n = ms << 80;
    for (i, b) in bytes.iter().enumerate() {
        n |= (*b as u128) << (72 - 8 * i);
    }
    let mut out = String::from("p-");
    for i in (0..26).rev() {
        out.push(CROCKFORD[((n >> (5 * i)) & 31) as usize] as char);
    }
    out
}

#[cfg(unix)]
fn urandom(buf: &mut [u8]) -> bool {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .is_ok()
}

#[cfg(not(unix))]
fn urandom(_buf: &mut [u8]) -> bool {
    false
}

/// What [`ensure`] did to the plan file, for `plan-panel`'s stderr.
pub enum Assigned {
    /// The file already had a live id.
    Kept(String),
    /// The file had none; it now has this one.
    New(String),
    /// The old id was approved with another body; the file has a new one.
    Replaced { old: String, new: String },
}

impl Assigned {
    pub fn id(&self) -> &str {
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

/// The approved body of `id`, when the person approved it.
fn approved_body(id: &str) -> Option<String> {
    let f = store()?.join("approved").join(format!("{id}.json"));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
    (v.get("schema")?.as_str()? == APPROVAL_SCHEMA)
        .then(|| v.get("body_sha")?.as_str().map(str::to_string))
        .flatten()
}

/// Gives `plan_file` a live `plan-id:` and maps its body to it.
///
/// The file is written only when its id is absent or retired, so a second
/// run leaves it byte-identical. The write follows `plan-apply`'s protocol:
/// the ledger lock, a re-read that refuses a file changed since it was
/// read, then an atomic rename. Errors name what to do next.
pub fn ensure(plan_file: &Path, text: &str) -> Result<Assigned, String> {
    let body = crate::plan_text::body_sha(text);
    let assigned = match read(text) {
        Some(id) => match approved_body(&id) {
            Some(b) if b != body => Assigned::Replaced {
                old: id,
                new: generate(&plan_file.display().to_string()),
            },
            _ => Assigned::Kept(id),
        },
        None => Assigned::New(generate(&plan_file.display().to_string())),
    };
    if !matches!(assigned, Assigned::Kept(_)) {
        let s = store().ok_or("no home directory, so the plan-review store cannot be located")?;
        crate::plan_review::private_dir(&s)
            .ok_or_else(|| format!("cannot create {}", s.display()))?;
        let _lock = crate::review_outcomes::lock(&s.join("outcomes.jsonl.lock"))
            .map_err(|e| format!("cannot lock the ledger: {e}"))?;
        let again = std::fs::read_to_string(plan_file)
            .map_err(|e| format!("cannot write plan-id: {}: {e}", plan_file.display()))?;
        if again != text {
            return Err(format!(
                "plan changed under plan-panel; nothing written. Re-read {0}, then run: greenpiste plan-panel {0}",
                plan_file.display()
            ));
        }
        greenpiste_guard::atomic::write_atomic(plan_file, &with_id(text, assigned.id()))
            .map_err(|e| format!("cannot write plan-id: {}: {e}", plan_file.display()))?;
    }
    remember(&body, assigned.id());
    Ok(assigned)
}

/// Maps `body_sha` to `id` under `ids/`. A failure costs only the map
/// entry (the row then reads the file's id when the body matches), so it is
/// reported, not fatal.
fn remember(body_sha: &str, id: &str) {
    let Some(dir) = store().map(|s| s.join("ids")) else {
        return;
    };
    if crate::plan_review::private_dir(&dir).is_none() {
        return;
    }
    let rec =
        serde_json::json!({"schema": ID_SCHEMA, "body_sha": body_sha, "plan_id": id}).to_string();
    if let Err(e) =
        greenpiste_guard::atomic::write_atomic(&dir.join(format!("{body_sha}.json")), &rec)
    {
        eprintln!(
            concat!(
                greenpiste_guard::bin!(),
                ": cannot record the plan-id of body {}: {}"
            ),
            &body_sha[..12.min(body_sha.len())],
            e
        );
    }
}

/// The id `plan-panel` mapped `body_sha` to.
pub fn id_for_body(body_sha: &str) -> Option<String> {
    let f = store()?.join("ids").join(format!("{body_sha}.json"));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
    if v.get("schema")?.as_str()? != ID_SCHEMA {
        return None;
    }
    v.get("plan_id")?
        .as_str()
        .filter(|id| valid(id))
        .map(str::to_string)
}

/// Records that the person approved `id` at `body_sha`: one file, written
/// atomically and without the ledger lock, so a busy ledger never loses it.
pub fn approve(id: &str, body_sha: &str) -> Result<(), String> {
    let dir = store().ok_or("no home directory")?.join("approved");
    crate::plan_review::private_dir(&dir)
        .ok_or_else(|| format!("cannot create {}", dir.display()))?;
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let rec = serde_json::json!({
        "schema": APPROVAL_SCHEMA, "plan_id": id, "body_sha": body_sha, "approved_at": at,
    })
    .to_string();
    greenpiste_guard::atomic::write_atomic(&dir.join(format!("{id}.json")), &rec)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_is_valid_and_distinct() {
        let a = generate("x");
        let b = generate("x");
        assert!(valid(&a), "{a}");
        assert_ne!(a, b);
    }

    #[test]
    fn read_and_write_the_front_matter() {
        assert_eq!(read("# T\n"), None);
        let id = generate("t");
        let made = with_id("# T\nbody\n", &id);
        assert_eq!(made, format!("---\nplan-id: {id}\n---\n# T\nbody\n"));
        assert_eq!(read(&made).as_deref(), Some(id.as_str()));
        let fm = "---\nstatus: active\nbranch: feat/x\n---\n# T\n";
        let added = with_id(fm, &id);
        assert_eq!(
            added,
            format!("---\nstatus: active\nbranch: feat/x\nplan-id: {id}\n---\n# T\n")
        );
        let other = generate("u");
        let replaced = with_id(&added, &other);
        assert_eq!(read(&replaced).as_deref(), Some(other.as_str()));
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
}
