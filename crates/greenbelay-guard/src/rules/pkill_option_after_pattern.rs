//! `pkill-option-after-pattern` — an option written after `pkill`'s pattern.
//!
//! ```sh
//! pkill -f "react-router-serve.*" -n
//! ```
//!
//! BSD `pkill`/`pgrep` (macOS, FreeBSD) read options with a getopt that
//! stops at the first operand, so every word after the pattern is another
//! PATTERN. Above, `-n` was meant as "newest only"; it became a second
//! pattern, and with `-f` it matched every process whose command line holds
//! `-n` — the `login … --noprofile --norc` of every Ghostty tab. On
//! 2026-11-25 that one call closed every terminal tab but the caller's own
//! (BSD pkill spares its ancestors) and ended fifteen Claude sessions.
//!
//! procps `pkill` (Linux) permutes its arguments, so there the same command
//! does what it says. `examine` reads the shape; `confirm` asks the `pgrep`
//! the command will run for `-V`, which procps answers and BSD refuses, and
//! stays silent on procps.

use crate::rules::{Confirmed, Context, Evidence, Examine, Finding, Rule, Stance, Trend};
use crate::shell::{Parsed, Simple};

pub const RULE: Rule = Rule {
    id: "pkill-option-after-pattern",
    prevents: "an option after pkill's pattern, which BSD pkill reads as another pattern and kills what it matches",
    irreversible: true,
    default_stance: Stance::Deny,
    max_stance: Stance::Deny,
    evidence: Evidence {
        // 2 in 124,189 calls (2026-10-12 → 11-25): the incident, and the
        // `pgrep` probe that diagnosed it. Rare, and irreversible when it hits.
        per_1000: 0.02,
        measured: "2026-11-25",
        trend: Trend::Rare,
    },
    examine: Examine::Legacy(examine),
    confirm: Some(confirm),
};

/// BSD getopt strings (FreeBSD `pgrep.c`, as shipped by macOS): the option
/// letters that take a value, in the same word or the next.
const VALUED: &[u8] = b"FGMNPUcdgjstu";

/// A pkill signal word, `-9` or `-TERM`/`-SIGKILL`: pkill takes it as its
/// first argument before getopt runs.
fn is_signal(text: &str) -> bool {
    let body = &text[1..];
    !body.is_empty()
        && (body.bytes().all(|c| c.is_ascii_digit())
            || body.bytes().all(|c| c.is_ascii_uppercase()))
}

/// The first word after the pattern that starts with `-`, read the way BSD
/// getopt reads the arguments.
fn misplaced(cmd: &Simple) -> Option<usize> {
    let program = cmd.program()?;
    let name = program.rsplit('/').next().unwrap_or(program);
    if name != "pkill" && name != "pgrep" {
        return None;
    }
    let args = cmd.args();
    let mut i = 0;
    if name == "pkill"
        && args
            .first()
            .is_some_and(|w| w.text.starts_with('-') && is_signal(&w.text))
    {
        i = 1;
    }
    // Options, up to the first operand.
    while let Some(w) = args.get(i) {
        let t = w.text.as_str();
        if t == "--" {
            i += 1;
            break;
        }
        if !t.starts_with('-') || t == "-" {
            break;
        }
        let bytes = &t.as_bytes()[1..];
        let mut takes_next = false;
        for (k, c) in bytes.iter().enumerate() {
            if VALUED.contains(c) {
                takes_next = k + 1 == bytes.len();
                break;
            }
        }
        i += if takes_next { 2 } else { 1 };
    }
    // `args[i]` is the pattern; every dash word after it is a pattern too.
    args.iter()
        .enumerate()
        .skip(i + 1)
        .find(|(_, w)| w.text.starts_with('-') && w.text.len() > 1)
        .map(|(j, _)| j)
}

fn examine(parsed: &Parsed) -> Option<Finding> {
    for cmd in parsed.clauses() {
        if cmd.opaque.is_some() {
            continue;
        }
        let Some(j) = misplaced(cmd) else {
            continue;
        };
        let word = &cmd.args()[j].text;
        return Some(Finding {
            reason: format!(
                "BSD `pkill`/`pgrep` stop reading options at the pattern, so `{word}` after it \
                 is a second pattern, not an option. With `-f` it matches every process whose \
                 command line contains `{word}` (`--noprofile` holds `-n`), and pkill signals \
                 them all: on 2026-11-25 `pkill -f \"…\" -n` closed every terminal tab."
            ),
            remedy: format!(
                "Put every option before the pattern: `pkill {word} -f '<pattern>'`. Run the \
                 same line as `pgrep -lf …` first to see what it matches, or kill by port \
                 (`lsof -ti :<port> | xargs kill`) or by pid."
            ),
            span: cmd.at..cmd.end,
        });
    }
    None
}

fn confirm(ctx: &Context, f: &Finding) -> Confirmed {
    let cwd = ctx.cwd_at(f.span.start);
    // `pgrep -V` only reports: procps prints its version, BSD refuses the
    // option. Neither signals anything.
    let procps = std::process::Command::new("pgrep")
        .arg("-V")
        .current_dir(if cwd.is_dir() { cwd.as_path() } else { ctx.cwd })
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("procps"))
        .unwrap_or(false);
    if procps {
        Confirmed::No("procps pkill permutes its arguments: the option is read as one")
    } else {
        Confirmed::Yes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::lex;

    fn fires(c: &str) -> bool {
        examine(&lex(c)).is_some()
    }

    #[test]
    fn an_option_after_the_pattern_fires() {
        assert!(fires(
            "pkill -f \"react-router-serve.*\" -n 2>/dev/null; lsof -ti :24101"
        ));
        assert!(fires("pgrep -f node -l"));
        assert!(fires("pkill -9 -f vite -x"));
        assert!(fires("sudo pkill -f 'storybook dev' -n"));
        assert!(fires("/usr/bin/pkill chrome --"));
        assert!(fires("pkill -f foo -- -n"));
    }

    #[test]
    fn options_before_the_pattern_are_silent() {
        assert!(!fires("pkill -n -f \"react-router-serve\""));
        assert!(!fires("pkill -f \"storybook dev -p 6127\""));
        assert!(!fires("pkill -TERM -f 'make plugin'"));
        assert!(!fires("pgrep -u root -lf sshd"));
        assert!(!fires("pgrep -P 1375 login"));
        assert!(!fires("pgrep -fl -t ttys001 fish"));
        assert!(!fires("pkill -f -- -n"));
        assert!(!fires("pkill chrome"));
        assert!(!fires("pgrep -x npm"));
        assert!(!fires("pkill -f foo bar"));
    }

    #[test]
    fn a_mention_is_not_a_call() {
        assert!(!fires("grep -rn 'pkill -f x -n' docs/"));
        assert!(!fires("echo pkill -f x -n"));
        assert!(!fires("ssh host 'pkill -f x -n'"));
    }
}
