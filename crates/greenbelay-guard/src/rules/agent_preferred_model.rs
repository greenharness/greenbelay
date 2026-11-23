//! `agent-preferred-model` — a subagent launched without the model its
//! definition prefers.
//!
//! An agent definition that says `model: inherit` runs on the session's model.
//! One that also declares `preferred-model: X` asks its launcher to pass
//! `model: "X"`. Like `relais-run-forks`, `examine` never fires (the backtester
//! sees only Bash); the hook's `PreToolUse` path for `Agent` and `Task` reads the
//! definition and calls [`judge`].

use crate::rules::{Evidence, Examine, Finding, Rule, Stance, Trend};
use crate::shell::Parsed;

pub const RULE: Rule = Rule {
    id: "agent-preferred-model",
    prevents: "A subagent launched on the session model where its definition prefers another",
    irreversible: false,
    default_stance: Stance::Advise,
    max_stance: Stance::Deny,
    evidence: Evidence {
        per_1000: 0.0,
        measured: "2026-11-23",
        trend: Trend::Rare,
    },
    examine: Examine::Legacy(examine),
    confirm: None,
};

/// The first line of a prompt that says the launch is a fallback.
pub const FALLBACK_MARKER: &str = "model-fallback:";

fn examine(_parsed: &Parsed) -> Option<Finding> {
    None
}

/// The two frontmatter values the rule reads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Definition {
    pub model: Option<String>,
    pub preferred_model: Option<String>,
}

fn unquote(raw: &str) -> &str {
    let raw = raw.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = raw
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.trim();
        }
    }
    raw
}

/// The `model:` and `preferred-model:` values between the first two `---`
/// fences; `None` when the text has no closed frontmatter.
pub fn parse_definition(text: &str) -> Option<Definition> {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let mut definition = Definition::default();
    for line in lines {
        let line = line.trim_end();
        if line == "---" {
            return Some(definition);
        }
        if let Some(value) = line.strip_prefix("model:") {
            definition.model = Some(unquote(value).to_string());
        } else if let Some(value) = line.strip_prefix("preferred-model:") {
            definition.preferred_model = Some(unquote(value).to_string());
        }
    }
    None
}

fn first_line_is_marker(prompt: &str) -> bool {
    let prompt = prompt.trim_start_matches('\u{feff}').trim_start();
    let first = prompt.lines().next().unwrap_or("");
    first.trim_end_matches('\r').starts_with(FALLBACK_MARKER)
}

/// The advice, when the definition inherits, prefers a model, the call names
/// none and the prompt is not a declared fallback.
pub fn judge(
    subagent_type: &str,
    definition_model: Option<&str>,
    preferred: Option<&str>,
    call_model: Option<&str>,
    prompt: &str,
) -> Option<String> {
    if definition_model? != "inherit" {
        return None;
    }
    let preferred = preferred.filter(|p| !p.is_empty())?;
    if call_model.is_some_and(|m| !m.trim().is_empty()) || first_line_is_marker(prompt) {
        return None;
    }
    Some(crate::decision::phrase(
        RULE.id,
        &format!(
            "{subagent_type} declares preferred-model: {preferred} but this call has no model, \
             so it would run on the session model."
        ),
        &format!(
            "Pass model: \"{preferred}\". Only after a model-availability failure, relaunch \
             without model and make the prompt's first line exactly: model-fallback: \
             {preferred} -> root (<why {preferred} was unavailable>)"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fires(model: Option<&str>, call: Option<&str>, prompt: &str) -> Option<String> {
        judge("impl-review", model, Some("opus"), call, prompt)
    }

    #[test]
    fn inherit_with_a_preference_and_no_model_fires() {
        assert_eq!(
            fires(Some("inherit"), None, "review this").unwrap(),
            "greenbelay/agent-preferred-model: impl-review declares preferred-model: opus but \
             this call has no model, so it would run on the session model. Pass model: \"opus\". \
             Only after a model-availability failure, relaunch without model and make the \
             prompt's first line exactly: model-fallback: opus -> root (<why opus was \
             unavailable>)"
        );
        assert!(fires(Some("inherit"), Some(""), "x").is_some());
    }

    #[test]
    fn an_explicit_model_passes() {
        for model in ["opus", "sonnet", "haiku", "inherit"] {
            assert_eq!(fires(Some("inherit"), Some(model), "x"), None);
        }
    }

    #[test]
    fn the_marker_passes_in_its_variants() {
        for prompt in [
            "model-fallback: opus -> root (rate limited)\nbody",
            "\u{feff}model-fallback: opus -> root (x)\nbody",
            "  \n\tmodel-fallback: opus -> root (x)",
            "model-fallback: opus -> root (x)\r\nbody",
            "model-fallback:",
        ] {
            assert_eq!(fires(Some("inherit"), None, prompt), None, "{prompt:?}");
        }
        assert!(fires(Some("inherit"), None, "body\nmodel-fallback: opus").is_some());
    }

    #[test]
    fn a_pinned_definition_passes() {
        assert_eq!(fires(Some("opus"), None, "x"), None);
        assert_eq!(fires(None, None, "x"), None);
    }

    #[test]
    fn no_preference_passes() {
        assert_eq!(judge("a", Some("inherit"), None, None, "x"), None);
        assert_eq!(judge("a", Some("inherit"), Some(""), None, "x"), None);
    }

    #[test]
    fn frontmatter_parses_quoted_values_and_crlf() {
        let text =
            "---\r\nname: r\r\nmodel: \"inherit\"\r\npreferred-model: 'opus'\r\n---\r\nbody\r\n";
        assert_eq!(
            parse_definition(text),
            Some(Definition {
                model: Some("inherit".into()),
                preferred_model: Some("opus".into())
            })
        );
        let plain = "---\nmodel: inherit\npreferred-model: opus\n---\n";
        assert_eq!(
            parse_definition(plain).unwrap().preferred_model.as_deref(),
            Some("opus")
        );
    }

    #[test]
    fn frontmatter_without_two_fences_is_malformed() {
        assert_eq!(parse_definition("model: inherit\n"), None);
        assert_eq!(parse_definition("---\nmodel: inherit\n"), None);
        let after = "---\nname: r\n---\nmodel: inherit\npreferred-model: opus\n";
        assert_eq!(parse_definition(after).unwrap(), Definition::default());
    }
}
