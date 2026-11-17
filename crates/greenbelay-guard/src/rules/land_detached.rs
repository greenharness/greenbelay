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
use crate::shell::{Connector, Parsed, Simple};

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
    let mut detached = detached.join(" ");
    // The land's own `cd <dir> &&` stays in front: the worker must run in the
    // same directory.
    let clauses = parsed.clauses();
    if let Some(cd) = clauses
        .iter()
        .zip(clauses.iter().skip(1))
        .find_map(|(before, after)| (after.at == cmd.at).then_some(before))
        .filter(|before| before.next == Some(Connector::AndAnd) && before.program() == Some("cd"))
    {
        let words: Vec<&str> = cd.words.iter().map(|w| w.raw.as_str()).collect();
        detached = format!("{} && {detached}", words.join(" "));
    }
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

/// Where the tool call runs: a background call is already off the turn.
#[derive(Clone, Copy)]
enum Run {
    Foreground,
    Background,
}

/// Speaks only for a foreground call near the cap, on a gate that has
/// `--detach`. `probe` reads the gate's version and is called only once the
/// background and timeout checks pass.
fn decide(run: Run, timeout_ms: u64, probe: impl FnOnce() -> Option<Version>) -> Confirmed {
    if let Run::Background = run {
        return Confirmed::No("the call already runs in the background");
    }
    if timeout_ms < NEAR_THE_CAP_MS {
        return Confirmed::No("the call's timeout is far from the tool's cap");
    }
    match probe() {
        Some(version) if gate_version::at_least(version, FIRST_VERSION_WITH_DETACH) => {
            Confirmed::Yes
        }
        _ => Confirmed::No("the gate on PATH is missing, unanswering or older than 4.9.0"),
    }
}

/// `program` as `PATH` resolves it, canonical, with its modification time in
/// seconds: `None` when it is not found. Like the shell, it skips empty or
/// relative entries and files without an execute bit.
fn binary_identity(program: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    let found = std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))?;
    let canonical = std::fs::canonicalize(&found).ok()?;
    let modified = std::fs::metadata(&canonical)
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(format!("{}@{modified}", canonical.display()))
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

/// The version of `program` on `PATH` when it has `land --detach`, remembered
/// on the session's record per program so the next hook process does not ask.
/// Only a version that was read is remembered: a missing or slow gate is
/// asked again.
fn gate_has_detach(session: &str, program: &str) -> Option<Version> {
    // Keyed on the binary PATH resolves and its modification time, so a PATH
    // change or an upgrade within the session asks again.
    let identity = binary_identity(program).unwrap_or_else(|| program.to_string());
    let capable = format!("land-detached {identity}-has-detach");
    let incapable = format!("land-detached {identity}-lacks-detach");
    if crate::lsp::was_told(session, &capable) {
        return Some(FIRST_VERSION_WITH_DETACH);
    }
    if crate::lsp::was_told(session, &incapable) {
        return None;
    }
    let version = gate_version::probe(program)?;
    let key = if gate_version::at_least(version, FIRST_VERSION_WITH_DETACH) {
        &capable
    } else {
        &incapable
    };
    let _ = crate::lsp::told(session, key); // unrecorded: the next hook process asks again
    Some(version)
}

fn confirm(context: &Context, finding: &Finding) -> Confirmed {
    confirm_with(context, finding, &|program| {
        gate_has_detach(context.session, program)
    })
}

fn confirm_with(
    context: &Context,
    _finding: &Finding,
    probe: &dyn Fn(&str) -> Option<Version>,
) -> Confirmed {
    let Some(program) = foreground_land(context.parsed).and_then(Simple::program) else {
        return Confirmed::No("the command no longer matches");
    };
    let run = if context.background {
        Run::Background
    } else {
        Run::Foreground
    };
    decide(run, context.timeout_ms(), || probe(program))
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
            .contains("`cd x && greengate land --detach --json --force`"));
        assert!(finding.remedy.contains("greengate land --wait"));
        assert!(finding.remedy.contains("Monitor"));
    }

    #[test]
    fn it_confirms_near_the_cap_on_a_gate_with_detach() {
        assert!(matches!(
            decide(Run::Foreground, 600_000, || Some((4, 9, 0))),
            Confirmed::Yes
        ));
        assert!(matches!(
            decide(Run::Foreground, 600_000, || Some((5, 0, 0))),
            Confirmed::Yes
        ));
    }

    #[test]
    fn it_stays_silent_on_a_short_timeout_an_old_gate_or_a_background_call() {
        assert!(matches!(
            decide(Run::Foreground, 120_000, || Some((4, 9, 0))),
            Confirmed::No(_)
        ));
        assert!(matches!(
            decide(Run::Foreground, 600_000, || Some((4, 8, 0))),
            Confirmed::No(_)
        ));
        assert!(matches!(
            decide(Run::Foreground, 600_000, || None),
            Confirmed::No(_)
        ));
        assert!(matches!(
            decide(Run::Background, 600_000, || Some((4, 9, 0))),
            Confirmed::No(_)
        ));
    }

    fn confirmed_through_context(
        run: Run,
        timeout_ms: Option<u64>,
        probe: &dyn Fn(&str) -> Option<Version>,
    ) -> Confirmed {
        let command = "greengate land --json";
        let parsed = lex(command);
        let finding = examine(&parsed).unwrap();
        let context = Context {
            platform: None,
            cwd: std::path::Path::new("/"),
            parsed: &parsed,
            background: matches!(run, Run::Background),
            timeout_ms,
            tool_use_id: "",
            transcript: None,
            agent_transcript: None,
            command,
            output: None,
            session: "",
            failed: false,
        };
        confirm_with(&context, &finding, probe)
    }

    #[test]
    fn a_background_call_never_reaches_the_probe() {
        let never = |_: &str| -> Option<Version> { panic!("the probe must not run") };
        assert!(matches!(
            confirmed_through_context(Run::Background, Some(600_000), &never),
            Confirmed::No(_)
        ));
    }

    #[test]
    fn a_short_timeout_never_reaches_the_probe() {
        let never = |_: &str| -> Option<Version> { panic!("the probe must not run") };
        assert!(matches!(
            confirmed_through_context(Run::Foreground, Some(120_000), &never),
            Confirmed::No(_)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn only_an_executable_file_is_a_gate() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("greenbelay-land-detached-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let gate = dir.join("greengate");
        std::fs::write(&gate, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&gate, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_executable(&gate));
        std::fs::set_permissions(&gate, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable(&gate));
        assert!(!is_executable(&dir));
        let _ = std::fs::remove_dir_all(&dir); // cleanup of a scratch dir: the asserts are taken
    }

    #[test]
    fn a_foreground_call_near_the_cap_probes_the_clauses_program() {
        let probe = |program: &str| (program == "greengate").then_some((4, 9, 0));
        assert!(matches!(
            confirmed_through_context(Run::Foreground, Some(600_000), &probe),
            Confirmed::Yes
        ));
    }
}
