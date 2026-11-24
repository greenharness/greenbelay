//! `work-owner` — which session holds a worktree or a programme ticket.
//!
//! Like `work-overlap`, `examine` never fires: the rule lives in the hook's
//! file and Bash paths, which claim a worktree for the session that writes in
//! it, record an observed `relais programme claim`, and journal the state of
//! every owner they look up. Everything it says is advice or a journal line.

use crate::activity::repo_name_of;
use crate::host_session::HostSession;
use crate::owner::{
    self, Claim, Contest, Identity, Outcome, Owner, Place, Reading, State, Store, TicketKey,
};
use crate::rules::{Evidence, Examine, Finding, Rule, Stance, Trend};
use crate::shell::Parsed;
use std::path::{Path, PathBuf};

pub const RULE: Rule = Rule {
    id: "work-owner",
    prevents: "A session taking over, or crediting work to, a worktree or ticket another live session owns",
    irreversible: false,
    default_stance: Stance::Advise,
    max_stance: Stance::Advise,
    evidence: Evidence {
        // Not yet measured: the owner records this rule reads start with it.
        per_1000: 0.0,
        measured: "2026-11-24",
        trend: Trend::Rare,
    },
    examine: Examine::Legacy(examine),
    confirm: None,
};

fn examine(_parsed: &Parsed) -> Option<Finding> {
    None
}

/// The git config key that turns the automatic worktree claim off.
pub const AUTO_CLAIM_KEY: &str = crate::config_key!("work-owner.auto-claim");

/// The journal word for an owner's state.
pub fn state_word(state: &State) -> &'static str {
    match state {
        State::Live => "live",
        State::Cleared { .. } => "cleared",
        State::Gone => "gone",
        State::Stale => "stale",
        State::Unknown => "unknown",
    }
}

/// A `relais programme claim <programme> <ticket> --dir <dir>` in a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgrammeClaim {
    pub programme: String,
    pub ticket: String,
    pub map_dir: PathBuf,
}

/// Every `relais programme claim <programme> <ticket> --dir <dir>` clause of
/// `command` whose words can be read literally; `<dir>` is resolved against
/// the directory the clause runs in.
pub fn programme_claims(command: &str, cwd: &Path) -> Vec<ProgrammeClaim> {
    use crate::shell::Resolved;
    if !command.contains("relais") {
        return Vec::new();
    }
    let parsed = crate::shell::lex(command);
    let context = crate::rules::Context {
        cwd,
        parsed: &parsed,
        background: false,
        timeout_ms: None,
        tool_use_id: "",
        transcript: None,
        agent_transcript: None,
        command,
        platform: None,
        output: None,
        session: "",
        failed: false,
    };
    let mut claims = Vec::new();
    for clause in parsed.judgeable() {
        if clause.program() != Some("relais") {
            continue;
        }
        let words = clause.args();
        let literal = |index: usize| match words.get(index).map(|w| parsed.word_value(clause.at, w))
        {
            Some(Resolved::Text(text)) => Some(text),
            _ => None,
        };
        if literal(0).as_deref() != Some("programme") || literal(1).as_deref() != Some("claim") {
            continue;
        }
        let mut operands: Vec<String> = Vec::new();
        let mut dir: Option<String> = None;
        let mut index = 2;
        while index < words.len() {
            let Some(text) = literal(index) else {
                operands.clear();
                break;
            };
            if text == "--dir" {
                dir = literal(index + 1);
                index += 2;
            } else if let Some(value) = text.strip_prefix("--dir=") {
                dir = Some(value.to_owned());
                index += 1;
            } else if text.starts_with('-') {
                index += 1;
            } else {
                operands.push(text);
                index += 1;
            }
        }
        if let (Some(programme), Some(ticket), Some(dir)) = (
            operands.first().cloned(),
            operands.get(1).cloned(),
            dir.filter(|_| operands.len() == 2),
        ) {
            claims.push(ProgrammeClaim {
                programme,
                ticket,
                map_dir: crate::rules::resolve(&context.cwd_at(clause.at), &dir),
            });
        }
    }
    claims
}

/// The ticket a `resume <repo> <programme> <ticket>[: …]` prompt names.
/// `None` for any other prompt, a driver's `resume <repo> <slug> drive: …`
/// included, and for a segment that is not a valid ticket key.
pub fn resume_target(prompt: &str) -> Option<TicketKey> {
    let mut words = prompt.split_whitespace();
    if words.next()? != "resume" {
        return None;
    }
    let repo = words.next()?;
    let programme = words.next()?;
    let ticket = words.next()?;
    if ticket == "drive:" || repo.ends_with(':') || programme.ends_with(':') {
        return None;
    }
    let ticket = ticket.strip_suffix(':').unwrap_or(ticket);
    TicketKey::new(repo, programme, ticket).ok()
}

/// Who owns the ticket a resume prompt names, as this session should hear it.
#[derive(Debug)]
pub enum ResumeOwner {
    /// Nothing to say; the word is the journal's.
    Silent(&'static str),
    /// This process held it before its `/clear`. `taken_over` when the ticket
    /// record now names this session.
    Continued {
        old: String,
        taken_over: bool,
    },
    /// Another live session holds it.
    OwnedBy {
        owner: Box<Owner>,
        worktree: Option<PathBuf>,
    },
    /// Its owner's process has cleared and runs another session now.
    Cleared {
        owner: Box<Owner>,
        now: String,
        name: String,
    },
    /// Live records name different sessions; nothing is taken over.
    Disagree(Vec<Owner>),
    Unknown(String),
}

impl ResumeOwner {
    pub fn journal_word(&self) -> &'static str {
        match self {
            ResumeOwner::Silent(word) => word,
            ResumeOwner::Continued {
                taken_over: true, ..
            } => "taken-over",
            ResumeOwner::Continued { .. } => "continued",
            ResumeOwner::OwnedBy { .. } => "live",
            ResumeOwner::Cleared { .. } => "cleared",
            ResumeOwner::Disagree(_) => "disagree",
            ResumeOwner::Unknown(_) => "unknown",
        }
    }
}

/// The owner of `key` for the session `session`, run by `caller` (`None`
/// when no registered process is above the hook).
///
/// The ticket record is authoritative when it exists. Only without one are
/// the worktree records on `prog/<programme>/<ticket>` read, those whose own
/// repository is named `key.repo()`; several sessions among them give
/// `Unknown`. The record is taken over only by this process's successor, and
/// only when no other live record disagrees.
pub fn resume_owner(
    store: &Store,
    key: &TicketKey,
    caller: Option<&HostSession>,
    session: &str,
    now: u64,
) -> ResumeOwner {
    let branch = format!("prog/{}/{}", key.programme(), key.ticket());
    let all = owner::worktree_owners(store, now);
    let on_branch: Vec<&(Owner, State)> = all
        .iter()
        .filter(|(held, _)| {
            held.branch == branch && repo_name_of(&held.repo_identity) == key.repo()
        })
        .collect();
    let holding = |state: &State| !matches!(state, State::Gone | State::Stale);
    let (record, state, from_ticket) = match owner::read_ticket(store, key, now) {
        Reading::Unknown => return ResumeOwner::Unknown("the ticket record is unreadable".into()),
        Reading::Held { owner, state } => (*owner, state, true),
        Reading::Free => {
            let held: Vec<&(Owner, State)> = on_branch
                .iter()
                .copied()
                .filter(|(_, state)| holding(state))
                .collect();
            let Some((first, state)) = held.first() else {
                return ResumeOwner::Silent("none");
            };
            if held
                .iter()
                .any(|(other, _)| other.identity.session != first.identity.session)
            {
                return ResumeOwner::Unknown(format!(
                    "{} worktree records on {branch} name different sessions",
                    held.len()
                ));
            }
            (first.clone(), state.clone(), false)
        }
    };
    if record.identity.session == session {
        return ResumeOwner::Silent("own");
    }
    let Some(caller) = caller else {
        if !holding(&state) {
            return ResumeOwner::Silent(state_word(&state));
        }
        return ResumeOwner::Unknown("no registered session above this hook".into());
    };
    let me = Identity {
        session: session.to_owned(),
        pid: caller.pid,
        started_at: caller.started_at,
        name: caller.name.clone(),
        reference: None,
    };
    let others: Vec<Owner> = on_branch
        .iter()
        .filter(|(held, state)| {
            matches!(state, State::Live)
                && held.identity.session != record.identity.session
                && held.identity.session != session
                && !held.identity.same_process(&me)
        })
        .map(|(held, _)| held.clone())
        .collect();
    if from_ticket && holding(&state) && !others.is_empty() {
        let mut owners = vec![record];
        owners.extend(others);
        return ResumeOwner::Disagree(owners);
    }
    if record.identity.same_process(&me) {
        let old = record.identity.session.clone();
        let taken_over = from_ticket && take_over(store, key, &record, me, now);
        return ResumeOwner::Continued { old, taken_over };
    }
    match state {
        State::Gone | State::Stale => ResumeOwner::Silent(state_word(&state)),
        State::Unknown => ResumeOwner::Unknown("host registry unreadable".into()),
        State::Cleared { now, name } => ResumeOwner::Cleared {
            owner: Box::new(record),
            now,
            name,
        },
        State::Live => {
            let worktree = all
                .iter()
                .find(|(held, _)| held.identity.session == record.identity.session)
                .and_then(|(held, _)| match &held.place {
                    Place::Worktree(path) => Some(path.clone()),
                    Place::Ticket { .. } => None,
                });
            ResumeOwner::OwnedBy {
                owner: Box::new(record),
                worktree,
            }
        }
    }
}

fn take_over(store: &Store, key: &TicketKey, record: &Owner, me: Identity, now: u64) -> bool {
    let Place::Ticket { map_dir } = &record.place else {
        return false;
    };
    let claim = Claim {
        repo_identity: record.repo_identity.clone(),
        branch: record.branch.clone(),
        identity: me,
    };
    matches!(
        owner::claim_ticket(store, key, map_dir, &claim, Contest::Respect, now),
        Ok(Outcome::TakenOver { .. })
    )
}

/// What the resume check says: the context for the model, and a note of at
/// most two lines for the person. `None` when it says nothing.
pub fn resume_text(
    key: &TicketKey,
    said: &ResumeOwner,
    now: u64,
) -> Option<(String, Option<String>)> {
    let ticket = key.ticket();
    match said {
        ResumeOwner::Silent(_) => None,
        ResumeOwner::Continued { old, .. } => Some((
            format!("you continue {ticket}, held by this process as {old} before /clear"),
            None,
        )),
        ResumeOwner::OwnedBy { owner, worktree } => {
            let seen = ago(now, owner.refreshed_at);
            let place = worktree
                .as_ref()
                .map(|path| format!("worktree {}, ", path.display()))
                .unwrap_or_default();
            let name = &owner.identity.name;
            Some((
                format!(
                    "{ticket} is owned by {} / {} ({place}seen {seen} ago): message it with SendMessage to \"{name}\" before any work; take it over only when it releases it or the person says so.",
                    owner.holder(),
                    owner.identity.session,
                ),
                Some(format!(
                    "WARNING: {ticket} is owned by {}, a live session; this session was told to message it before any work.",
                    owner.holder()
                )),
            ))
        }
        ResumeOwner::Cleared { owner, now, name } => Some((
            format!(
                "{} held {ticket}; its process {name} has cleared and now runs {now}, which has none of this history: do not credit it.",
                owner.identity.session
            ),
            Some(format!(
                "WARNING: {ticket}'s owner {} has cleared; {name} now runs {}, which has none of the work.",
                short(&owner.identity.session),
                short(now)
            )),
        )),
        ResumeOwner::Disagree(owners) => {
            let named: Vec<String> = owners
                .iter()
                .map(|owner| format!("{} / {}", owner.holder(), owner.identity.session))
                .collect();
            Some((
                format!(
                    "{ticket} has owner records that disagree: {}. Message them with SendMessage before any work, and take nothing over.",
                    named.join("; ")
                ),
                Some(format!(
                    "WARNING: {ticket} has owner records that disagree: {}.",
                    owners
                        .iter()
                        .map(Owner::holder)
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            ))
        }
        ResumeOwner::Unknown(why) => Some((
            format!(
                "ownership of {ticket} is unknown ({why}); run `{} board` before any work.",
                crate::bin!()
            ),
            None,
        )),
    }
}

fn short(uuid: &str) -> &str {
    uuid.get(..8).unwrap_or(uuid)
}

fn ago(now: u64, then: u64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    crate::activity::ago(
        UNIX_EPOCH + Duration::from_secs(now),
        UNIX_EPOCH + Duration::from_secs(then),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_programme_claim_is_read_with_its_dir() {
        let claims = programme_claims(
            "relais programme claim follow-ups t02 --dir docs/programmes/follow-ups",
            Path::new("/w/repo"),
        );
        assert_eq!(
            claims,
            vec![ProgrammeClaim {
                programme: "follow-ups".to_owned(),
                ticket: "t02".to_owned(),
                map_dir: PathBuf::from("/w/repo/docs/programmes/follow-ups"),
            }]
        );
    }

    #[test]
    fn other_relais_commands_and_unreadable_words_claim_nothing() {
        let cwd = Path::new("/w");
        for command in [
            "relais programme frontier --dir d",
            "relais programme claim p t",
            "relais programme claim p --dir d",
            "relais programme claim p $T --dir d",
            "git commit -m 'relais programme claim p t --dir d'",
        ] {
            assert_eq!(programme_claims(command, cwd), vec![], "{command}");
        }
    }

    #[test]
    fn a_resume_prompt_names_its_ticket_and_a_drive_names_none() {
        let key = resume_target("resume greenbelay follow-ups t02-x: Three sessions").unwrap();
        assert_eq!(key.shown(), "greenbelay/follow-ups/t02-x");
        assert_eq!(
            resume_target("  resume decisions follow-ups f107\n")
                .unwrap()
                .shown(),
            "decisions/follow-ups/f107"
        );
        for prompt in [
            "resume greenbelay follow-ups drive: t01 t02",
            "resume greenbelay follow-ups",
            "please resume greenbelay follow-ups t02",
            "resume greenbelay ../x t02",
            "implement docs/plans/x.md in greenbelay",
        ] {
            assert!(resume_target(prompt).is_none(), "{prompt}");
        }
    }
}
