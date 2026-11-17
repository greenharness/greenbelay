//! `land-detached` — a foreground land that the tool's own clock may kill.
//!
//! A land runs the repository's pre-push gate inside its push, and that gate
//! can take most of the Bash tool's ten-minute timeout. A call killed at the
//! timeout leaves a land half-done and reports only the kill. greengate 4.9.0
//! adds `land --detach`: the land runs as a worker and writes a receipt, and
//! `land --wait <receipt>` follows it, so the call returns at once.
//!
//! `examine` keys on the command alone (a land without `--detach`, `--wait` or
//! `--dry-run`); `confirm` is where the fact lives: a foreground call with a
//! timeout near the cap, and a greengate on `PATH` that has `--detach`.

use crate::gate_version::{self, Version};
use crate::rules::{Confirmed, Context, Evidence, Examine, Finding, Rule, Stance, Trend};
use crate::shell::{Parsed, Simple};

pub const RULE: Rule = Rule {
    id: "land-detached",
    prevents: "A foreground land killed at the tool's timeout while its pre-push gate still runs",
    irreversible: false,
    // Advises from the start: it refuses nothing, and the failure it names is
    // a land cut off mid-push with nothing said about why.
    default_stance: Stance::Advise,
    max_stance: Stance::Deny,
    evidence: Evidence {
        per_1000: 0.0,
        measured: "2026-11-17",
        trend: Trend::Rare,
    },
    examine: Examine::Legacy(examine),
    confirm: Some(confirm),
};

/// The first greengate with `land --detach`.
const FIRST_VERSION_WITH_DETACH: Version = (4, 9, 0);

/// The shortest call timeout at which a land is at risk: the tool's cap is
/// 600 000 ms, and a gate that takes most of it is what this rule is about.
const NEAR_THE_CAP_MS: u64 = 540_000;

/// The land clause this rule speaks about, if any.
fn foreground_land(parsed: &Parsed) -> Option<&Simple> {
    parsed.judgeable().find(|cmd| {
        crate::land::is_land(cmd)
            && !cmd.has_flag("--detach")
            && !cmd.has_flag("--wait")
            && !cmd.is_dry_run()
    })
}

fn examine(parsed: &Parsed) -> Option<Finding> {
    let cmd = foreground_land(parsed)?;
    let program = cmd.program()?;
    let land_at = cmd
        .words
        .iter()
        .position(|w| !w.quoted && w.text == "land")?;
    let mut detached = vec![program, "land", "--detach"];
    detached.extend(cmd.words[land_at + 1..].iter().map(|w| w.raw.as_str()));
    let detached = detached.join(" ");
    Some(Finding {
        reason: "a land runs the pre-push gate inside its push, and the Bash tool kills a \
                 foreground call at its timeout and reports only the kill: a land cut off \
                 there is left half-done."
            .to_string(),
        remedy: format!(
            "Run `{detached}`: it starts the land as a worker and returns with the \
             receipt's path. Then follow it with `{program} land --wait <receipt path>` \
             under Monitor, which ends when the land does."
        ),
        span: cmd.at..cmd.end,
    })
}

/// Pure: speaks only for a foreground call near the cap, on a gate that has
/// `--detach`.
fn decide(background: bool, timeout_ms: u64, gate: Option<Version>) -> Confirmed {
    if background {
        return Confirmed::No("the call already runs in the background");
    }
    if timeout_ms < NEAR_THE_CAP_MS {
        return Confirmed::No("the call's timeout is far from the tool's cap");
    }
    if !gate_version::at_least(gate, FIRST_VERSION_WITH_DETACH) {
        return Confirmed::No("the gate on PATH is missing, unanswering or older than 4.9.0");
    }
    Confirmed::Yes
}

fn confirm(context: &Context, _finding: &Finding) -> Confirmed {
    let Some(program) = foreground_land(context.parsed).and_then(Simple::program) else {
        return Confirmed::No("the command no longer matches");
    };
    // The probe is the costly step: ask it only when the rest holds.
    let probed = if context.background || context.timeout_ms() < NEAR_THE_CAP_MS {
        None
    } else {
        gate_version::probe(program)
    };
    decide(context.background, context.timeout_ms(), probed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::lex;

    fn fires(command: &str) -> bool {
        examine(&lex(command)).is_some()
    }

    #[test]
    fn a_foreground_land_has_the_shape() {
        assert!(fires("greengate land --json"));
        assert!(fires("amont land --json"));
        assert!(fires("cd /x/wt && greengate land --tag v1.2.3"));
    }

    #[test]
    fn a_detached_waiting_or_dry_land_does_not() {
        assert!(!fires("greengate land --detach --json"));
        assert!(!fires("greengate land --wait /r/receipt.json"));
        assert!(!fires("greengate land --dry-run --json"));
        assert!(!fires("git push origin feat"));
    }

    #[test]
    fn the_remedy_is_the_calls_own_command_detached() {
        let finding = examine(&lex("cd x && greengate land --json --force")).unwrap();
        assert!(finding
            .remedy
            .contains("`greengate land --detach --json --force`"));
        assert!(finding.remedy.contains("greengate land --wait"));
        assert!(finding.remedy.contains("Monitor"));
    }

    #[test]
    fn it_confirms_near_the_cap_on_a_gate_with_detach() {
        assert!(matches!(
            decide(false, 600_000, Some((4, 9, 0))),
            Confirmed::Yes
        ));
        assert!(matches!(
            decide(false, 600_000, Some((5, 0, 0))),
            Confirmed::Yes
        ));
    }

    #[test]
    fn it_stays_silent_on_a_short_timeout_an_old_gate_or_a_background_call() {
        assert!(matches!(
            decide(false, 120_000, Some((4, 9, 0))),
            Confirmed::No(_)
        ));
        assert!(matches!(
            decide(false, 600_000, Some((4, 8, 0))),
            Confirmed::No(_)
        ));
        assert!(matches!(decide(false, 600_000, None), Confirmed::No(_)));
        assert!(matches!(
            decide(true, 600_000, Some((4, 9, 0))),
            Confirmed::No(_)
        ));
    }
}
