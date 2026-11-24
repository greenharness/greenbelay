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

/// The longest prompt the clear-relay plugin submits, in UTF-16 units: the
/// relay measures a JavaScript string's `.length`.
pub const MAX_PROMPT_UNITS: usize = 2_000;

/// What the line trims, on both sides of it, here and in the relay: the
/// language `trim`s differ (Rust strips U+0085, JavaScript U+FEFF).
const TRIM: &[char] = &[' ', '\t', '\r', '\n'];

/// What an answer's last line is to the clear-relay plugin. The relay
/// (`plugin/clear-relay/hooks/register.ts`, `handoffLine`) reads the same
/// grammar, and `plugin/clear-relay/hooks/handoff-cases.ts` pins both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffLine {
    /// The last line does not start with [`MARKER`].
    Absent,
    /// `safe to /clear; next: `<prompt>``: the relay arms on the prompt.
    Next(String),
    /// `safe to /clear; parked: <line>`: the relay submits nothing.
    Parked,
    /// A line that starts with [`MARKER`] the relay does not arm on.
    NearMiss(NearMiss),
}

impl HandoffLine {
    /// The relay acts on it: a `next:` prompt or a park.
    pub fn is_armed(&self) -> bool {
        matches!(self, Self::Next(_) | Self::Parked)
    }
}

/// Why a line that starts with [`MARKER`] does not arm the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NearMiss {
    EmptyParked,
    NotNextOrParked,
    EmptyPrompt,
    MissingBackticks,
    InnerBacktick,
    /// The prompt's length, in UTF-16 units.
    Overlong(usize),
}

impl NearMiss {
    /// The stable id, the relay's `reason` and the journal's.
    pub fn id(self) -> &'static str {
        match self {
            Self::EmptyParked => "empty-parked",
            Self::NotNextOrParked => "not-next-or-parked",
            Self::EmptyPrompt => "empty-prompt",
            Self::MissingBackticks => "missing-backticks",
            Self::InnerBacktick => "inner-backtick",
            Self::Overlong(_) => "overlong",
        }
    }

    /// What is wrong, for a sentence that ends "does not arm on it: …".
    pub fn reason(self) -> String {
        match self {
            Self::EmptyParked => "the parked line after `parked:` is empty".to_string(),
            Self::NotNextOrParked => {
                format!("`{MARKER}` is followed by neither `; next: ` nor `; parked: `")
            }
            Self::EmptyPrompt => "the prompt is empty".to_string(),
            Self::MissingBackticks => {
                "the prompt after `next: ` is not wrapped in backticks".to_string()
            }
            Self::InnerBacktick => "the prompt holds a backtick".to_string(),
            Self::Overlong(units) => {
                format!("the prompt is {units} UTF-16 units, over the 2,000 limit")
            }
        }
    }
}

/// The handoff line of `answer`: its last line that is not empty once
/// trimmed. The checks run in order and the first that fails names the
/// near-miss: the marker; a park (`; parked: <text>`, an empty one
/// `empty-parked`); `; next: ` with its one space (`; next:` alone is
/// `empty-prompt`); backticks opening and closing the rest, read untrimmed;
/// a prompt that is not blank, holds no backtick and is at most
/// [`MAX_PROMPT_UNITS`] long.
pub fn handoff_line(answer: &str) -> HandoffLine {
    let Some(last) = answer
        .split('\n')
        .map(|line| line.trim_matches(TRIM))
        .rfind(|line| !line.is_empty())
    else {
        return HandoffLine::Absent;
    };
    let Some(rest) = last.strip_prefix(MARKER) else {
        return HandoffLine::Absent;
    };
    let near = HandoffLine::NearMiss;
    if let Some(parked) = rest.strip_prefix("; parked:") {
        return if parked.trim_matches(TRIM).is_empty() {
            near(NearMiss::EmptyParked)
        } else if parked.starts_with(' ') {
            HandoffLine::Parked
        } else {
            near(NearMiss::NotNextOrParked)
        };
    }
    if rest == "; next:" {
        return near(NearMiss::EmptyPrompt);
    }
    let Some(after) = rest.strip_prefix("; next: ") else {
        return near(NearMiss::NotNextOrParked);
    };
    if after.trim_matches(TRIM).is_empty() {
        return near(NearMiss::EmptyPrompt);
    }
    let Some(prompt) = after
        .strip_prefix('`')
        .and_then(|inner| inner.strip_suffix('`'))
    else {
        return near(NearMiss::MissingBackticks);
    };
    if prompt.trim_matches(TRIM).is_empty() {
        return near(NearMiss::EmptyPrompt);
    }
    if prompt.contains('`') {
        return near(NearMiss::InnerBacktick);
    }
    let units = prompt.encode_utf16().count();
    if units > MAX_PROMPT_UNITS {
        return near(NearMiss::Overlong(units));
    }
    HandoffLine::Next(prompt.to_string())
}

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

/// What the size stance is resolved from, each key already parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeStanceInputs {
    /// `$GREENBELAY_OFF` set or `greenbelay.enabled=false`.
    pub off: bool,
    /// The session is in this rule's shadow arm.
    pub shadow: bool,
    /// `greenbelay.stance`, the floor every rule reads, when set.
    pub global_stance: Option<Stance>,
    /// `greenbelay.clear-at-boundary.stance`, when set.
    pub rule_stance: Option<Stance>,
    /// `greenbelay.clear-at-boundary.size-stance`, when set.
    pub size_stance: Option<Stance>,
}

/// The size kind's stance: `size-stance` (default [`SIZE_STANCE_DEFAULT`]),
/// forced to `observe` by the kill switches, the shadow arm, or an explicit
/// `observe` in the global or the rule's own stance key, and capped by the
/// rule's ceiling. The rule's compiled `advise` does not lower it.
///
/// Its order is not [`crate::stance::resolve`]'s: there the rule's key
/// overrides `greenbelay.stance`, here an `observe` in either wins, so a
/// global `observe` quiets the size kind even under a rule key of `deny`.
/// It errs quiet on purpose: the size kind is new and asks for a handoff.
pub fn size_stance_of(inputs: SizeStanceInputs) -> Stance {
    let quieted = [inputs.global_stance, inputs.rule_stance].contains(&Some(Stance::Observe));
    if inputs.off || inputs.shadow || quieted {
        return Stance::Observe;
    }
    inputs
        .size_stance
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
    // An unset key reads as "", which parses to `None`; a bad value is
    // reported by `enumerated_or` and also reads as unset.
    let configured = |key: &str| Stance::parse(config::enumerated_or(key, ALLOWED, ""));
    let stance = size_stance_of(SizeStanceInputs {
        off,
        shadow: false,
        global_stance: configured(crate::stance::KEY_STANCE),
        rule_stance: configured(&crate::stance::key_for(&RULE)),
        size_stance: configured(KEY_SIZE_STANCE),
    });
    if stance != Stance::Observe && crate::shadow::drops(RULE.id, RULE.irreversible) {
        return Stance::Observe;
    }
    stance
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relay's case table: one grammar for both readers.
    const HANDOFF_CASES: &str =
        include_str!("../../../../plugin/clear-relay/hooks/handoff-cases.ts");

    fn handoff_cases() -> Vec<serde_json::Value> {
        let block = HANDOFF_CASES
            .split_once("// handoff-cases:begin\n")
            .and_then(|(_, rest)| rest.split_once("// handoff-cases:end"))
            .map(|(block, _)| block)
            .expect("handoff-cases.ts holds its begin and end markers");
        serde_json::from_str(block).expect("the block between the markers is strict JSON")
    }

    /// The case's answer and, for a `next` case, the prompt it submits.
    fn case_answer(case: &serde_json::Value) -> (String, Option<String>) {
        if let Some(repeat) = case.get("next_repeat") {
            let unit = repeat["unit"].as_str().unwrap();
            let count = usize::try_from(repeat["count"].as_u64().unwrap()).unwrap();
            let tail = repeat["tail"].as_str().unwrap_or("");
            let prompt = format!("{}{tail}", unit.repeat(count));
            return (format!("{MARKER}; next: `{prompt}`"), Some(prompt));
        }
        let answer = case["answer"].as_str().unwrap().to_string();
        let prompt = case["prompt"].as_str().map(str::to_string);
        (answer, prompt)
    }

    #[test]
    fn every_handoff_case_reads_as_the_relay_reads_it() {
        let cases = handoff_cases();
        assert!(cases.len() >= 30, "{} cases", cases.len());
        for case in &cases {
            let name = case["name"].as_str().unwrap();
            let (answer, prompt) = case_answer(case);
            let got = handoff_line(&answer);
            match case["kind"].as_str().unwrap() {
                "next" => assert_eq!(got, HandoffLine::Next(prompt.unwrap()), "{name}"),
                "parked" => assert_eq!(got, HandoffLine::Parked, "{name}"),
                "absent" => assert_eq!(got, HandoffLine::Absent, "{name}"),
                "near-miss" => {
                    let HandoffLine::NearMiss(miss) = got else {
                        panic!("{name}: {got:?} is not a near-miss");
                    };
                    assert_eq!(miss.id(), case["reason"].as_str().unwrap(), "{name}");
                }
                other => panic!("{name}: unknown kind {other}"),
            }
        }
    }

    #[test]
    fn an_overlong_prompt_counts_utf16_units() {
        let prompt = "😀".repeat(1001);
        assert_eq!(
            handoff_line(&format!("{MARKER}; next: `{prompt}`")),
            HandoffLine::NearMiss(NearMiss::Overlong(2002))
        );
        assert!(NearMiss::Overlong(2002)
            .reason()
            .contains("2002 UTF-16 units"));
    }

    #[test]
    fn only_a_next_or_a_park_is_armed() {
        assert!(HandoffLine::Next("x".into()).is_armed());
        assert!(HandoffLine::Parked.is_armed());
        assert!(!HandoffLine::Absent.is_armed());
        assert!(!HandoffLine::NearMiss(NearMiss::EmptyPrompt).is_armed());
    }

    fn inputs() -> SizeStanceInputs {
        SizeStanceInputs {
            off: false,
            shadow: false,
            global_stance: None,
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
            rule_stance: Some(Stance::Advise),
            size_stance: Some(Stance::Deny),
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
                rule_stance: Some(Stance::Observe),
                size_stance: Some(Stance::Deny),
                ..inputs()
            },
            SizeStanceInputs {
                global_stance: Some(Stance::Observe),
                size_stance: Some(Stance::Deny),
                ..inputs()
            },
        ] {
            assert_eq!(size_stance_of(forced), Stance::Observe, "{forced:?}");
        }
    }

    #[test]
    fn the_size_stance_key_sets_the_stance() {
        for setting in [Stance::Observe, Stance::Advise, Stance::Deny] {
            let set = SizeStanceInputs {
                size_stance: Some(setting),
                ..inputs()
            };
            assert_eq!(size_stance_of(set), setting);
        }
    }
}
