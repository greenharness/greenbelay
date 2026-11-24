//! `work-owner` — which session holds a worktree or a programme ticket.
//!
//! Like `work-overlap`, `examine` never fires: the rule lives in the hook's
//! file and Bash paths, which claim a worktree for the session that writes in
//! it, record an observed `relais programme claim`, and journal the state of
//! every owner they look up. Everything it says is advice or a journal line.

use crate::owner::State;
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
}
