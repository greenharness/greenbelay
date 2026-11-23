//! `clear-at-boundary`: a turn ended at a boundary, with nothing in flight,
//! and the main chain's context is past 200k tokens, so a `/clear` would cost
//! less than carrying on.
//!
//! It fires on `Stop`, not on a shell command, so `examine` never matches a
//! command; the judgement is `greenbelay_process::clear_boundary`, called
//! from the hook's `Stop` arm. The rule is declared here so its stance
//! resolves, graduates and reports like every other rule's.
//!
//! Default `advise`, ceiling `deny`: a `Stop` continuation is the only way to
//! have the handoff written, so `deny` sends it, `advise` tells the person
//! instead, and `observe` only journals. It ships at `advise` because its
//! backtest missed two bars (25 of 31 compacted sessions advised before their
//! first compaction against 28; 46 of 140 advices followed by an editing
//! turn against at most 10%); `deny` waits on the Outcome's 7- and 14-day
//! counts.
//!
//! Its size kind (ADR-0083) has its own keys, read here: `size-tokens`, the
//! threshold past which an idle `Stop` hands off mid-phase, and
//! `size-stance`, default `observe` until its live evidence graduates it,
//! which the rule's compiled `advise` does not lower.

use crate::rules::{Evidence, Examine, Finding, Rule, Stance, Trend};
use crate::shell::Parsed;

/// What the handoff's last line says; the handoff step prints this constant.
pub const MARKER: &str = "safe to /clear";

pub const RULE: Rule = Rule {
    id: "clear-at-boundary",
    prevents:
        "A turn ending at a boundary past 200k tokens of context without a handoff and a clear",
    irreversible: false,
    default_stance: Stance::Advise,
    max_stance: Stance::Deny,
    evidence: Evidence {
        per_1000: 0.0,
        measured: "2026-11-13",
        trend: Trend::Rare,
    },
    examine: Examine::Legacy(examine),
    confirm: None,
};

fn examine(_: &Parsed) -> Option<Finding> {
    None
}

/// The size boundary's threshold, in tokens (ADR-0083).
pub const KEY_SIZE_TOKENS: &str = crate::config_key!("clear-at-boundary.size-tokens");
/// The size kind's own stance: `deny`, `advise` or `observe`.
pub const KEY_SIZE_STANCE: &str = crate::config_key!("clear-at-boundary.size-stance");
/// The size boundary without configuration.
pub const SIZE_TOKENS_DEFAULT: u64 = 400_000;
/// The lowest threshold accepted: the command kind's own size, below which a
/// clear does not pay for itself.
pub const SIZE_TOKENS_FLOOR: u64 = 200_000;
/// The size kind's stance without configuration. It ships at `observe`: the
/// 30-day replay (2026-11-24) fired 2.71 times a day, but 13 of 84 fires
/// (15.5%, the bar is 10%) came while a task launched outside the replayed
/// transcript was still running, a fact one transcript cannot hold; the
/// follow-up re-judges `deny` on live observe evidence.
pub const SIZE_STANCE_DEFAULT: Stance = Stance::Observe;

const ALLOWED: &[&str] = &["observe", "advise", "deny"];

/// The configured size threshold: `size-tokens`, at least
/// [`SIZE_TOKENS_FLOOR`]; a bad value is reported on stderr and reads as
/// [`SIZE_TOKENS_DEFAULT`].
pub fn size_tokens() -> u64 {
    crate::gitconfig::integer_at_least_or(KEY_SIZE_TOKENS, SIZE_TOKENS_FLOOR, SIZE_TOKENS_DEFAULT)
}

/// What the size stance is resolved from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeStanceInputs<'a> {
    /// `$GREENBELAY_OFF` set or `greenbelay.enabled=false`.
    pub off: bool,
    /// The session is in this rule's shadow arm.
    pub shadow: bool,
    /// `greenbelay.clear-at-boundary.stance`, when set.
    pub rule_stance: Option<&'a str>,
    /// `greenbelay.clear-at-boundary.size-stance`, when set.
    pub size_stance: Option<&'a str>,
}

/// The size kind's stance: `size-stance` (default [`SIZE_STANCE_DEFAULT`]), forced to
/// `observe` by the kill switches, the shadow arm or an explicit rule stance
/// of `observe`, and capped by the rule's ceiling. The rule's compiled
/// `advise` does not lower it.
pub fn size_stance_of(inputs: SizeStanceInputs) -> Stance {
    if inputs.off || inputs.shadow || inputs.rule_stance == Some("observe") {
        return Stance::Observe;
    }
    inputs
        .size_stance
        .and_then(Stance::parse)
        .unwrap_or(SIZE_STANCE_DEFAULT)
        .min(RULE.max_stance)
}

/// [`size_stance_of`] over the live configuration. Each key is a `git
/// config` read, so it is called only once a Stop is past the size.
pub fn size_stance() -> Stance {
    use crate::gitconfig as config;
    let off =
        crate::stance::switched_off() || !config::boolean_or(crate::stance::KEY_ENABLED, true);
    if off {
        return Stance::Observe;
    }
    let set = |key: &str| Some(config::enumerated_or(key, ALLOWED, "")).filter(|v| !v.is_empty());
    let rule_key = crate::stance::key_for(&RULE);
    let rule_stance = set(&rule_key);
    let size_stance = set(KEY_SIZE_STANCE);
    let stance = size_stance_of(SizeStanceInputs {
        off,
        shadow: false,
        rule_stance,
        size_stance,
    });
    if stance != Stance::Observe && crate::shadow::drops(RULE.id, RULE.irreversible) {
        return Stance::Observe;
    }
    stance
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> SizeStanceInputs<'static> {
        SizeStanceInputs {
            off: false,
            shadow: false,
            rule_stance: None,
            size_stance: None,
        }
    }

    #[test]
    fn the_size_stance_ships_at_observe() {
        assert_eq!(size_stance_of(inputs()), Stance::Observe);
    }

    #[test]
    fn the_compiled_advise_does_not_lower_the_size_stance() {
        assert_eq!(RULE.default_stance, Stance::Advise);
        let advise = SizeStanceInputs {
            rule_stance: Some("advise"),
            size_stance: Some("deny"),
            ..inputs()
        };
        assert_eq!(size_stance_of(advise), Stance::Deny);
    }

    #[test]
    fn switches_and_an_explicit_observe_force_observe() {
        for forced in [
            SizeStanceInputs {
                off: true,
                ..inputs()
            },
            SizeStanceInputs {
                shadow: true,
                ..inputs()
            },
            SizeStanceInputs {
                rule_stance: Some("observe"),
                size_stance: Some("deny"),
                ..inputs()
            },
        ] {
            assert_eq!(size_stance_of(forced), Stance::Observe, "{forced:?}");
        }
    }

    #[test]
    fn the_size_stance_key_sets_the_stance() {
        for (value, stance) in [
            ("observe", Stance::Observe),
            ("advise", Stance::Advise),
            ("deny", Stance::Deny),
        ] {
            let set = SizeStanceInputs {
                size_stance: Some(value),
                ..inputs()
            };
            assert_eq!(size_stance_of(set), stance);
        }
    }
}
