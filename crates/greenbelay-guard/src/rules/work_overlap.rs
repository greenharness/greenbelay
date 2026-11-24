//! `work-overlap` — a session's first Edit or Write in a repository, while
//! another session was lately working in the same repository.
//!
//! Like `agent-guidance-written`, `examine` never fires — the backtester sees
//! no Edit or Write calls — and the rule lives in the hook's file path, which
//! asks [`crate::activity`] who else is live and calls [`phrase`]. Told once
//! per session and repository.

use crate::activity::Peer;
use crate::names::BIN;
use crate::rules::{Evidence, Examine, Finding, Rule, Stance, Trend};
use crate::shell::Parsed;

pub const RULE: Rule = Rule {
    id: "work-overlap",
    prevents: "Two sessions editing the same repository without either knowing about the other",
    irreversible: false,
    default_stance: Stance::Advise,
    max_stance: Stance::Advise,
    evidence: Evidence {
        // Not yet measured: the activity record this rule reads starts with it.
        per_1000: 0.0,
        measured: "2026-10-31",
        trend: Trend::Rare,
    },
    examine: Examine::Legacy(examine),
    confirm: None,
};

fn examine(_parsed: &Parsed) -> Option<Finding> {
    None
}

/// One budget for every plan lookup of one piece of advice.
const PLAN_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

/// The title of the first active plan the peer's branch added under
/// `docs/plans` relative to the repository's default branch, read in the
/// peer's worktree; `None` for a peer on the default branch, a lookup that
/// failed, or the `deadline`.
fn peer_plan(peer: &Peer, deadline: std::time::Instant) -> Option<String> {
    if on_default_branch_by_files(peer) {
        return None;
    }
    let (default, default_name) = crate::git::default_ref(&peer.worktree, deadline)?;
    if peer.branch == default_name {
        return None;
    }
    crate::git::active_added_plans(&peer.worktree, &default, "HEAD", deadline)
        .into_iter()
        .next()
        .map(|plan| plan.title)
}

/// Whether the peer's branch is its repository's default; see
/// [`is_default_branch`].
fn on_default_branch_by_files(peer: &Peer) -> bool {
    let Some(common) = crate::activity::resolve(&peer.worktree).map(|place| place.repo_identity)
    else {
        return false;
    };
    is_default_branch(&common, &peer.branch)
}

/// Whether `branch` is the default of the repository whose common dir is
/// `common`, read from files with no git process: `refs/remotes/origin/HEAD`,
/// else the usual names. `false` means "not known", and the caller asks git.
///
/// Without `origin/HEAD`, `master` is the default only when no `main` exists:
/// `git::default_ref` prefers `main`, and this shortcut must agree with it.
pub fn is_default_branch(common: &std::path::Path, branch: &str) -> bool {
    let origin_head = std::fs::read_to_string(common.join("refs/remotes/origin/HEAD")).ok();
    match origin_head
        .as_deref()
        .and_then(|text| text.trim().strip_prefix("ref: refs/remotes/origin/"))
    {
        Some(name) => branch == name,
        None => match branch {
            "main" => true,
            "master" => !ref_exists_by_files(common, "main"),
            _ => false,
        },
    }
}

/// Whether `refs/heads/<name>` or `refs/remotes/origin/<name>` exists, loose
/// or packed, in the common dir. A ref that cannot be read counts as present,
/// so a doubt leaves the answer to git.
fn ref_exists_by_files(common: &std::path::Path, name: &str) -> bool {
    let refs = [
        format!("refs/heads/{name}"),
        format!("refs/remotes/origin/{name}"),
    ];
    if refs.iter().any(|r| common.join(r).exists()) {
        return true;
    }
    match std::fs::read_to_string(common.join("packed-refs")) {
        Ok(packed) => packed.lines().any(|line| {
            line.split_once(' ')
                .is_some_and(|(_, r)| refs.iter().any(|want| want == r))
        }),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// What the host registry and the owner records say about a peer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerOwner {
    /// The peer's SendMessage name.
    pub name: Option<String>,
    /// The short ref its owner record carries.
    pub reference: Option<String>,
    pub standing: Standing,
}

/// Whether the peer is the session that owns its worktree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Standing {
    /// No owner record names the peer.
    #[default]
    Unrecorded,
    Live,
    /// The peer's process now runs another session.
    Cleared {
        now: String,
    },
    /// Gone, stale or unknown: nothing to say beyond the name.
    Other,
}

/// The owners of `peers`, in order: the name from the host registry (or the
/// owner record when the registry no longer knows the peer's UUID), and the
/// standing of the peer's worktree record when it names the peer.
/// The owner each peer is, read from `store` and the registry it names;
/// `None` (no store) leaves every peer unnamed.
pub fn peer_owners(
    store: Option<&crate::owner::Store>,
    peers: &[Peer],
    now_seconds: u64,
) -> Vec<PeerOwner> {
    peers
        .iter()
        .map(|peer| {
            let Some(store) = store else {
                return PeerOwner::default();
            };
            let mut owner = PeerOwner {
                name: crate::host_session::lookup_in(&store.registry, &peer.session)
                    .map(|host| host.name)
                    .filter(|name| !name.is_empty()),
                ..PeerOwner::default()
            };
            let crate::owner::Reading::Held {
                owner: record,
                state,
            } = crate::owner::read_worktree(store, &peer.worktree, now_seconds)
            else {
                return owner;
            };
            if record.identity.session != peer.session {
                return owner;
            }
            owner.name = owner.name.or(Some(record.identity.name.clone()));
            owner.reference = record.identity.reference.clone();
            owner.standing = match state {
                crate::owner::State::Live => Standing::Live,
                crate::owner::State::Cleared { now, .. } => Standing::Cleared { now },
                _ => Standing::Other,
            };
            owner
        })
        .collect()
}

/// The one-line advice, naming every live peer (at most three, freshest first)
/// with its name beside the UUID and the plan each one's branch added; the
/// plan lookups share one 2 s budget and a peer reached after it is named
/// without a plan. `owners` is [`peer_owners`], in the order of `peers`.
pub fn phrase(
    repo_name: &str,
    peers: &[Peer],
    owners: &[PeerOwner],
    now: std::time::SystemTime,
) -> String {
    const NAMED: usize = 3;
    let deadline = std::time::Instant::now() + PLAN_BUDGET;
    let none = PeerOwner::default();
    let named: Vec<String> = peers
        .iter()
        .zip(owners.iter().chain(std::iter::repeat(&none)))
        .take(NAMED)
        .map(|(peer, owner)| {
            let plan = peer_plan(peer, deadline)
                .map(|title| format!(", active plan \"{title}\""))
                .unwrap_or_default();
            let who = match (&owner.name, &owner.reference) {
                (Some(name), Some(reference)) => format!("`{name}` [{reference}] / "),
                (Some(name), None) => format!("`{name}` / "),
                (None, _) => String::new(),
            };
            let standing = match &owner.standing {
                Standing::Cleared { now } => format!(
                    ", cleared since: its process now runs `{now}`, which has none of this work, so do not credit it"
                ),
                Standing::Live | Standing::Unrecorded | Standing::Other => String::new(),
            };
            format!(
                "session {who}`{}` (worktree `{}`, branch `{}`{plan}, last seen {} ago{standing})",
                peer.session,
                peer.worktree.display(),
                peer.branch,
                crate::activity::ago(now, peer.last_seen),
            )
        })
        .collect();
    let more = peers.len().saturating_sub(NAMED);
    let tail = if more > 0 {
        format!(" and {more} more")
    } else {
        String::new()
    };
    let live_owner = peers
        .iter()
        .zip(owners)
        .take(NAMED)
        .find(|(_, owner)| owner.standing == Standing::Live)
        .and_then(|(_, owner)| owner.name.as_deref());
    let remedy = match live_owner {
        Some(name) => format!(
            "It owns that worktree and is live: SendMessage to \"{name}\" before editing there, or ask the person. `{BIN} board --repo {repo_name}` lists every session."
        ),
        None => format!(
            "Run `{BIN} board --repo {repo_name}`, then message that session (SendMessage) or ask the person."
        ),
    };
    crate::decision::phrase(
        RULE.id,
        &format!(
            "another session is working in this repository: {}{tail}.",
            named.join("; ")
        ),
        &remedy,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn the_advice_names_the_session_worktree_branch_age_and_remedy() {
        let now = UNIX_EPOCH + Duration::from_secs(10_000);
        let peer = Peer {
            session: "s-other".to_string(),
            worktree: PathBuf::from("/w/feat"),
            branch: "feat/x".to_string(),
            last_seen: now - Duration::from_secs(25 * 60),
        };
        let text = phrase("amont", &[peer], &[], now);
        assert!(!text.contains("active plan"), "{text}");
        for part in [
            "s-other",
            "/w/feat",
            "feat/x",
            "25m",
            "greenbelay board --repo amont",
            "SendMessage",
        ] {
            assert!(text.contains(part), "{part}: {text}");
        }
    }

    #[test]
    fn a_live_owner_is_named_with_its_ref_and_messaged() {
        let now = UNIX_EPOCH + Duration::from_secs(10_000);
        let peer = Peer {
            session: "uuid-a".to_string(),
            worktree: PathBuf::from("/w/feat"),
            branch: "feat/x".to_string(),
            last_seen: now,
        };
        let owner = PeerOwner {
            name: Some("perso-3b".to_string()),
            reference: Some("417dc6".to_string()),
            standing: Standing::Live,
        };
        let text = phrase("amont", std::slice::from_ref(&peer), &[owner], now);
        for part in [
            "`perso-3b` [417dc6] / `uuid-a`",
            "SendMessage to \"perso-3b\"",
        ] {
            assert!(text.contains(part), "{part}: {text}");
        }
        let cleared = PeerOwner {
            name: Some("perso-3b".to_string()),
            reference: None,
            standing: Standing::Cleared {
                now: "uuid-b".to_string(),
            },
        };
        let text = phrase("amont", &[peer], &[cleared], now);
        assert!(text.contains("cleared since"), "{text}");
        assert!(text.contains("do not credit it"), "{text}");
        assert!(!text.contains("SendMessage to"), "{text}");
    }
}
