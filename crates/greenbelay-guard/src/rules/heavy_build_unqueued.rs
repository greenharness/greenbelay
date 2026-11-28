//! `heavy-build-unqueued` — a heavy build or test run outside the host slot
//! queue.
//!
//! ```sh
//! cargo test                                   # beside every other session's
//! greengate slot run --now -- cargo test      # skips the queue on purpose
//! ```
//!
//! Every heavy job on the machine queues on amont's host slots (decisions
//! ADR-0049). A build or test suite started bare runs beside whatever the
//! other sessions are running, and the host thrashes until gates time out.
//!
//! Heavy is [`runner_words`] from `test-output-trimmed`, plus `cargo build`,
//! `cargo check`, `xcodebuild` and `swift build`. A clause wrapped as
//! `amont slot run -- <cmd>`, `amont run <check>` or `amont rehearse` is
//! queued and is not judged; `slot run --now -- <cmd>` is the
//! exception, because it launches the command at once, unqueued (greengate
//! 5.0; `--on-timeout=run`, which older versions honour, does the same after
//! a wait, and is still recognised).
//!
//! ## `confirm` is where the fact lives
//!
//! The rule speaks only when queueing is possible: the hook's environment has
//! no `GREENHARNESS_HOST_SLOT` (the command already holds a slot otherwise), and
//! the gate on `PATH` (`greengate`, else `amont`) says `--version` 1.62.0 or
//! newer (the first with `slot run`). A host with only `greengate` confirms.
//! The advice names that gate, and the `--now` sentence only when it is 5.0 or
//! newer (4.13 lacks the flag). A version that was read is remembered for the
//! session; the probe has a short deadline. The slot files are never read:
//! `<gate> slots` is the query.

use crate::rules::test_output_trimmed::{exempt_words, runner_words};
use crate::rules::{Confirmed, Context, Evidence, Examine, Finding, Rule, Stance, Trend};
use crate::shell::{Parsed, Simple, Word};

pub const RULE: Rule = Rule {
    id: "heavy-build-unqueued",
    prevents: "A build or test suite run beside every other session's, unqueued, thrashing the host until gates time out",
    irreversible: false,
    default_stance: Stance::Advise,
    max_stance: Stance::Advise,
    evidence: Evidence {
        per_1000: 0.0,
        measured: "2026-10-29",
        trend: Trend::Rare,
    },
    examine: Examine::Legacy(examine),
    confirm: Some(confirm),
};

/// The first `amont` with `slot run`.
const FIRST_VERSION_WITH_SLOTS: crate::gate_version::Version = (1, 62, 0);

/// Set by `amont slot run` in the environment of the command it launches.
/// The variable a slot holder exports to what it runs (greengate 4.0 and
/// relais). greengate 3.x's `AMONT_HOST_SLOT` is not read.
const HELD_SLOT_VARIABLE: &str = "GREENHARNESS_HOST_SLOT";

/// The first gate with `slot run --now`.
const FIRST_VERSION_WITH_NOW: crate::gate_version::Version = (5, 0, 0);

/// The gate the advice names, and what it can do.
struct Gate {
    name: &'static str,
    has_now: bool,
}

/// What a replay (no host to ask) assumes.
const ASSUMED_GATE: Gate = Gate {
    name: "amont",
    has_now: true,
};

/// The most words of the command the advice repeats.
const ECHOED_WORDS: usize = 8;

struct Heavy<'a> {
    program: &'a str,
    args: &'a [Word],
    launch: Launch<'a>,
}

/// How a heavy clause comes to run unqueued.
enum Launch<'a> {
    /// Started bare, outside any queue.
    Bare,
    /// `slot run --now`: at once, skipping the queue (greengate 5.0).
    Now { gate: &'a str },
    /// `slot run --on-timeout=run`: queued first, unqueued when no slot frees
    /// (older versions; greengate 5.0 refuses it).
    OnTimeoutRun { gate: &'a str },
}

/// Build commands beyond the runners `test-output-trimmed` knows.
fn builds(program: &str, words: &[Word]) -> bool {
    let operands: Vec<&str> = words
        .iter()
        .filter(|w| !w.quoted)
        .map(|w| w.text.as_str())
        .collect();
    let first = operands
        .iter()
        .copied()
        .find(|a| !a.starts_with('-') && !a.starts_with('+'));
    match program {
        "cargo" => matches!(first, Some("build" | "check")),
        "swift" => first == Some("build"),
        "xcodebuild" => !operands
            .iter()
            .any(|a| matches!(*a, "-version" | "-list" | "-showsdks" | "-help")),
        _ => false,
    }
}

pub(crate) fn is_heavy(program: &str, words: &[Word]) -> bool {
    !exempt_words(words) && (runner_words(program, words) || builds(program, words))
}

/// The heavy command a clause starts unqueued, if any.
fn unqueued(cmd: &Simple) -> Option<Heavy<'_>> {
    let program = cmd.program()?;
    if crate::gate::is_gate_program(program) {
        return slot_run_fallback(cmd);
    }
    is_heavy(program, cmd.args()).then(|| Heavy {
        program,
        args: cmd.args(),
        launch: Launch::Bare,
    })
}

/// `slot run --now -- <heavy>`, or the older `--on-timeout=run`: the command
/// runs unqueued, at once or when no slot frees in time.
fn slot_run_fallback(cmd: &Simple) -> Option<Heavy<'_>> {
    let args = cmd.args();
    let separator = args.iter().position(|w| !w.quoted && w.text == "--")?;
    let own: Vec<&str> = args[..separator]
        .iter()
        .filter(|w| !w.quoted)
        .map(|w| w.text.as_str())
        .collect();
    let mut operands = own.iter().copied().filter(|a| !a.starts_with('-'));
    if operands.next() != Some("slot") || operands.next() != Some("run") {
        return None;
    }
    let gate = cmd.program()?;
    let gate = gate.rsplit('/').next().unwrap_or(gate);
    let launch = if own.contains(&"--now") {
        Launch::Now { gate }
    } else if own.iter().enumerate().any(|(i, a)| {
        *a == "--on-timeout=run" || (*a == "--on-timeout" && own.get(i + 1) == Some(&"run"))
    }) {
        Launch::OnTimeoutRun { gate }
    } else {
        return None;
    };
    let (program, rest) = args[separator + 1..].split_first()?;
    if program.quoted || !is_heavy(&program.text, rest) {
        return None;
    }
    Some(Heavy {
        program: &program.text,
        args: rest,
        launch,
    })
}

fn examine(parsed: &Parsed) -> Option<Finding> {
    finding(parsed, &ASSUMED_GATE)
}

fn finding(parsed: &Parsed, found: &Gate) -> Option<Finding> {
    for cmd in parsed.judgeable() {
        let Some(heavy) = unqueued(cmd) else {
            continue;
        };
        let mut shown = vec![heavy.program];
        shown.extend(heavy.args.iter().take(ECHOED_WORDS).map(|w| w.raw.as_str()));
        if heavy.args.len() > ECHOED_WORDS {
            shown.push("…"); // the cut is marked: the queued form is not the whole command
        }
        let shown = shown.join(" ");
        let reason = format!(
            "`{shown}` starts a heavy build or test run outside the host slot queue, \
             so it runs beside every other session's and the machine thrashes until \
             gates time out."
        );
        let queued = |gate: &str| {
            format!(
                "`{gate} slot run -- {shown}` waits at most 90 s (`--max-wait`) for a slot, \
                 and exit 75 means no slot freed and the command was not launched. \
                 `{gate} slots` shows who holds them."
            )
        };
        let remedy = match heavy.launch {
            Launch::Bare => {
                let gate = found.name;
                let now = if found.has_now {
                    " Retry later, or pass `--now` to run it at once, unqueued, past a \
                     stalled line when you accept the load."
                } else {
                    " Retry later."
                };
                format!(
                    "Run it as `{gate} slot run -- {shown}`: it waits at most 90 s \
                     (`--max-wait`) for a slot; exit 75 means no slot freed, or the line \
                     stalled, and the command was not launched.{now} `{gate} slots` shows \
                     who holds them."
                )
            }
            Launch::Now { gate } => format!(
                "`--now` runs it at once, unqueued. Drop it to queue: {}",
                queued(gate)
            ),
            Launch::OnTimeoutRun { gate } => format!(
                "`--on-timeout=run` runs it unqueued when no slot frees in time \
                 (greengate 5.0 refuses it). Drop it to queue: {}",
                queued(gate)
            ),
        };
        return Some(Finding {
            reason,
            remedy,
            span: cmd.at..cmd.end,
        });
    }
    None
}

fn confirm(context: &Context, found: &Finding) -> Confirmed {
    if std::env::var_os(HELD_SLOT_VARIABLE).is_some() {
        return Confirmed::No("the hook runs inside a host slot already");
    }
    let Some(gate) = slot_gate(context.session) else {
        return Confirmed::No("no gate on PATH is answering, or it is older than 1.62.0");
    };
    // The advice names the gate that is here: build it again for that gate.
    match finding(context.parsed, &gate) {
        Some(own) if own.span == found.span => Confirmed::YesSaying {
            floor: None,
            ceiling: None,
            reason: own.reason,
            excerpt: context
                .command
                .get(found.span.clone())
                .unwrap_or_default()
                .to_string(),
            remedy: Some(own.remedy),
        },
        _ => Confirmed::Yes,
    }
}

/// The gate on `PATH` (`greengate`, else `amont`) when it has `slot run`,
/// remembered on the session's record so the next hook process does not ask.
/// Only a version that was read is remembered: a missing, failing or slow gate
/// (a loaded host is when the probe times out) is asked again next time, never
/// recorded as too old.
fn slot_gate(session: &str) -> Option<Gate> {
    const CAPABLE: &str = "heavy-build-unqueued amont-has-slots";
    const INCAPABLE: &str = "heavy-build-unqueued amont-lacks-slots";
    const NOW: &str = "heavy-build-unqueued gate-has-now";
    let program = crate::gate::gate_binary();
    let name = if program == "greengate" {
        "greengate"
    } else {
        "amont"
    };
    if crate::lsp::was_told(session, CAPABLE) {
        let has_now = crate::lsp::was_told(session, NOW);
        return Some(Gate { name, has_now });
    }
    if crate::lsp::was_told(session, INCAPABLE) {
        return None;
    }
    let version = crate::gate_version::probe(program)?;
    let capable = crate::gate_version::at_least(version, FIRST_VERSION_WITH_SLOTS);
    let has_now = crate::gate_version::at_least(version, FIRST_VERSION_WITH_NOW);
    let _ = crate::lsp::told(session, if capable { CAPABLE } else { INCAPABLE }); // unrecorded: the next hook process asks again
    if capable && has_now {
        let _ = crate::lsp::told(session, NOW);
    }
    capable.then_some(Gate { name, has_now })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::lex;

    fn fires(command: &str) -> bool {
        examine(&lex(command)).is_some()
    }

    #[test]
    fn a_bare_heavy_command_fires() {
        for c in [
            "cargo test",
            "cd x && pnpm test",
            "xcodebuild -scheme App build",
            "cargo build --release",
            "cargo check",
            "swift build",
            "amont slot run --on-timeout=run -- cargo test",
            "amont slot run --on-timeout run -- cargo test",
            "amont slot run --now -- cargo test",
            "greengate slot run --now -- cargo test -p foo",
        ] {
            assert!(fires(c), "{c}");
        }
    }

    #[test]
    fn a_queued_or_light_command_does_not() {
        for c in [
            "amont slot run -- cargo test",
            "amont slot run --weight 2 -- cargo test",
            "amont slot run --on-timeout=fail -- cargo test",
            "amont run cargo-test",
            "amont rehearse --wait",
            "cargo fmt",
            "cargo test --help",
            "cargo test --no-run",
            "swift --version",
            "xcodebuild -version",
            "git status",
        ] {
            assert!(!fires(c), "{c}");
        }
    }

    #[test]
    fn the_advice_names_the_queue_and_the_exit_code() {
        let finding = examine(&lex("cd x && cargo test -p foo")).unwrap();
        assert!(finding
            .remedy
            .contains("amont slot run -- cargo test -p foo"));
        assert!(finding.remedy.contains("90 s"));
        assert!(finding.remedy.contains("75"));
        assert!(finding.remedy.contains("amont slots"));
    }

    #[test]
    fn the_advice_for_a_bare_command_names_now() {
        let finding = examine(&lex("cargo test")).unwrap();
        assert!(finding.remedy.contains("`--now`"));
        assert!(!finding.remedy.contains("--on-timeout"));
        assert!(finding.remedy.contains("stalled"));
    }

    #[test]
    fn the_advice_for_now_says_what_it_skips_and_how_to_queue() {
        let finding = examine(&lex("greengate slot run --now -- cargo test -p foo")).unwrap();
        assert!(finding.remedy.contains("`--now` runs it at once, unqueued"));
        assert!(finding
            .remedy
            .contains("greengate slot run -- cargo test -p foo"));
        assert!(finding.remedy.contains("greengate slots"));
        assert!(!finding.remedy.contains("--on-timeout"));
    }

    #[test]
    fn the_advice_for_the_older_flag_does_not_talk_about_now() {
        for c in [
            "amont slot run --on-timeout=run -- cargo test",
            "amont slot run --on-timeout run -- cargo test",
        ] {
            let finding = examine(&lex(c)).unwrap();
            assert!(!finding.remedy.contains("`--now` runs"), "{c}");
            assert!(
                finding
                    .remedy
                    .contains("--on-timeout=run` runs it unqueued"),
                "{c}"
            );
            assert!(
                finding.remedy.contains("amont slot run -- cargo test"),
                "{c}"
            );
        }
    }

    #[test]
    fn the_bare_advice_names_the_gate_that_is_here() {
        let parsed = lex("cargo test");
        let greengate = Gate {
            name: "greengate",
            has_now: true,
        };
        let remedy = finding(&parsed, &greengate).unwrap().remedy;
        assert!(
            remedy.contains("`greengate slot run -- cargo test`"),
            "{remedy}"
        );
        assert!(remedy.contains("`greengate slots`"), "{remedy}");
        assert!(!remedy.contains("amont"), "{remedy}");
        assert!(remedy.contains("`--now`"), "{remedy}");
    }

    #[test]
    fn a_gate_without_now_is_not_told_to_pass_it() {
        let parsed = lex("cargo test");
        let old = Gate {
            name: "greengate",
            has_now: false,
        };
        let remedy = finding(&parsed, &old).unwrap().remedy;
        assert!(!remedy.contains("--now"), "{remedy}");
        assert!(remedy.contains("Retry later."), "{remedy}");
    }

    #[test]
    fn a_long_command_is_cut_with_a_mark() {
        let nine = examine(&lex("cargo test a b c d e f g h")).unwrap();
        assert!(
            nine.remedy.contains("test a b c d e f g …`"),
            "{}",
            nine.remedy
        );
        assert!(!nine.remedy.contains("g h"), "{}", nine.remedy);
        let eight = examine(&lex("cargo test a b c d e f g")).unwrap();
        assert!(!eight.remedy.contains('…'), "{}", eight.remedy);
    }

    #[test]
    fn the_first_now_version_is_after_the_first_slot_version() {
        assert!(FIRST_VERSION_WITH_NOW > FIRST_VERSION_WITH_SLOTS);
        assert!(!crate::gate_version::at_least(
            (4, 13, 0),
            FIRST_VERSION_WITH_NOW
        ));
    }

    #[test]
    fn the_first_slot_version_is_a_slot_version() {
        let read = crate::gate_version::parse_version("amont 2.0.0").unwrap();
        assert!(crate::gate_version::at_least(
            read,
            FIRST_VERSION_WITH_SLOTS
        ));
    }

    #[test]
    fn greengate_gets_the_verdict_amont_gets() {
        for c in [
            "greengate slot run --on-timeout=run -- cargo test",
            "greengate slot run --on-timeout run -- cargo test",
        ] {
            assert!(fires(c), "{c}");
        }
        for c in [
            "greengate slot run -- cargo test",
            "greengate slot run --weight 2 -- cargo test",
            "greengate slot run --on-timeout=fail -- cargo test",
            "greengate run cargo-test",
            "greengate rehearse --wait",
        ] {
            assert!(!fires(c), "{c}");
        }
    }
}
