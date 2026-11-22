//! `whole-file-dump` — a file poured into the tool result with no window.
//!
//! ```sh
//! cat app/lib/landscapeDoc.ts          # 56 KB into context
//! sed -n '1,400p' crates/x/src/lib.rs   # four hundred lines, most unread
//! ```
//!
//! Measured over 41,700 tool calls (2026-09-09): files opened through
//! `cat`, `sed -n` and `head` were 31.5% of every byte the tools returned,
//! four times the Read tool's share, and the largest single family in the
//! transcripts. Nothing fails; the bytes simply arrive, and every later
//! turn of the session pays for them again. The Read tool returns the same
//! text with line numbers, a size cap and `offset`/`limit`.
//!
//! `examine` fires on the shape — a reader whose output goes nowhere but
//! the tool result (see [`crate::rules::dump`]). `confirm` looks at the file:
//! a whole dump of anything over 4 KB, or a bounded read of a hundred lines
//! or more, is what this rule is about; a `cat` of a 300-byte config is not.

use crate::rules::dump::{dumps, Dump, Extent};
use crate::rules::{Confirmed, Context, Evidence, Examine, Finding, Rule, Stance, Trend};
use crate::shell::Parsed;
use std::path::{Path, PathBuf};

pub const RULE: Rule = Rule {
    id: "whole-file-dump",
    prevents: "A file poured into the tool result with no window, most of it unread",
    irreversible: false,
    default_stance: Stance::Advise,
    max_stance: Stance::Deny,
    evidence: Evidence {
        // 7,278 dumps in 41,700 calls; 1,571 of them unbounded over a file
        // the Read tool would have windowed.
        per_1000: 174.5,
        measured: "2026-09-09",
        trend: Trend::Flat(4),
    },
    examine: Examine::Legacy(examine),
    confirm: Some(confirm),
};

/// Below this a dump costs less than the advice about it.
const SMALL: u64 = 4096;
/// A bounded read this long is a dump with extra steps.
const MANY_LINES: u64 = 100;

/// Whether a dump is what this rule is about; `len` is the file's size when
/// it is known, and a whole dump of an unknown size counts.
fn qualifies(extent: Extent, len: Option<u64>) -> bool {
    match extent {
        Extent::Whole => len.is_none_or(|len| len >= SMALL),
        Extent::Lines(n) => n >= MANY_LINES,
        Extent::Bytes(n) => n >= 2 * SMALL,
    }
}

fn pour_reason(path: &str) -> String {
    format!(
        "`{path}` is poured whole into the tool result: no line numbers, no size cap, and \
         every later turn of this session carries it again. Files opened this way were \
         31% of all tool-result bytes measured."
    )
}

fn pour_remedy(path: &str) -> String {
    format!(
        "Open it with the Read tool, with `offset`/`limit` around the part you need — \
         `grep -n '<anchor>' {path} | head` finds the line first. `cat` is fine inside a pipe \
         or a heredoc."
    )
}

fn examine(parsed: &Parsed) -> Option<Finding> {
    let all = dumps(parsed);
    let mut shape = all.iter().filter(|d| qualifies(d.extent, None));
    let first = shape.next()?;
    let last = shape.next_back().unwrap_or(first);
    Some(Finding {
        reason: pour_reason(&first.path),
        remedy: pour_remedy(&first.path),
        span: first.at..last.end,
    })
}

/// Past this the whole dumps of a command may be refused (once: the hook lets
/// the same command through when it is repeated); a smaller total, or a
/// bounded read, only ever advises. The same threshold as
/// `read-unbounded-large`, measured for the Read tool's whole reads
/// (deliberate-output plan, Phase 3).
pub const REFUSABLE: u64 = 8 * 1024;

/// The file a dump names: where it is, resolved against `cwd` (the directory
/// its clause runs in). `None` when a `~/` path has no `HOME` to expand against.
fn resolve(cwd: &Path, dump: &Dump) -> Option<PathBuf> {
    if dump.path.starts_with('/') {
        Some(PathBuf::from(&dump.path))
    } else if let Some(rest) = dump.path.strip_prefix("~/") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(rest))
    } else {
        Some(cwd.join(&dump.path))
    }
}

/// A file the refusal is bound to: where it is, and the directory its clause
/// runs in.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub path: PathBuf,
    pub cwd: PathBuf,
}

/// A dump whose file is there to be looked at.
struct Seen {
    shown: String,
    path: PathBuf,
    cwd: PathBuf,
    extent: Extent,
    len: u64,
    /// The bytes this dump counts toward a refusal: its length when it is a
    /// whole dump of a regular file (a FIFO, a device or a directory is never
    /// counted, and never opened), not exempt, and not on the right of `||`
    /// (it may print, but is not certain to). `None` when it is not charged.
    charged: Option<u64>,
}

/// Files a refusal is not made for: plans, diffs and patches, and persisted
/// tool output. Media is not exempt here, as it is for Read: `cat x.png` pours
/// raw bytes into the context.
fn exempt(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("diff") || ext.eq_ignore_ascii_case("patch") {
        return true;
    }
    let parts: Vec<_> = path.components().map(|c| c.as_os_str()).collect();
    if parts
        .windows(2)
        .any(|w| w[1] == "plans" && (w[0] == ".claude" || w[0] == "docs"))
    {
        return true;
    }
    crate::rules::persisted_output_dump::is_persisted(&path.to_string_lossy())
}

/// The byte spans of every command on the right of a `||`; `None` when the
/// command's `||` structure could not be read.
fn alternatives(src: &str) -> Option<Vec<std::ops::Range<usize>>> {
    use crate::analysis::ir::{AndOrOp, Cmd};
    fn walk(cmd: &Cmd) -> Vec<std::ops::Range<usize>> {
        match cmd {
            Cmd::AndOr { first, rest, .. } => {
                let mut ranges = walk(first);
                for (op, next) in rest {
                    if *op == AndOrOp::Or {
                        ranges.push(next.span());
                    }
                    ranges.extend(walk(next));
                }
                ranges
            }
            Cmd::Seq { items, .. } => items.iter().flat_map(|item| walk(&item.cmd)).collect(),
            Cmd::Pipeline { cmds, .. } => cmds.iter().flat_map(walk).collect(),
            Cmd::If {
                arms, otherwise, ..
            } => {
                let mut ranges: Vec<_> = arms
                    .iter()
                    .flat_map(|(condition, body)| {
                        let mut both = walk(condition);
                        both.extend(walk(body));
                        both
                    })
                    .collect();
                if let Some(otherwise_body) = otherwise {
                    ranges.extend(walk(otherwise_body));
                }
                ranges
            }
            Cmd::Case { arms, .. } => arms.iter().flat_map(|arm| walk(&arm.body)).collect(),
            Cmd::For { body, .. }
            | Cmd::ArithFor { body, .. }
            | Cmd::Group { body, .. }
            | Cmd::Subshell { body, .. }
            | Cmd::FuncDef { body, .. }
            | Cmd::Redirected { body, .. } => walk(body),
            Cmd::Loop { cond, body, .. } => {
                let mut ranges = walk(cond);
                ranges.extend(walk(body));
                ranges
            }
            Cmd::Simple(_) | Cmd::Arith { .. } | Cmd::Cond { .. } | Cmd::Unsupported { .. } => {
                Vec::new()
            }
        }
    }
    crate::analysis::frontend::parse(src)
        .ok()
        .map(|cmd| walk(&cmd))
}

/// Every dump of the command whose file is there, looked at with `stat` alone,
/// and whether the command's `||` structure could be read.
fn seen(context: &Context) -> (Vec<Seen>, bool) {
    let alternative_spans = alternatives(context.command);
    let readable = alternative_spans.is_some();
    let alternative_spans = alternative_spans.unwrap_or_default();
    let files = dumps(context.parsed)
        .into_iter()
        .filter_map(|dump| {
            let cwd = context.cwd_at(dump.at);
            let path = resolve(&cwd, &dump)?;
            let meta = std::fs::metadata(&path).ok()?;
            let alternative = alternative_spans
                .iter()
                .any(|span| dump.at < span.end && span.start < dump.end);
            let charged = (matches!(dump.extent, Extent::Whole)
                && meta.is_file()
                && !exempt(&path)
                && !alternative)
                .then_some(meta.len());
            Some(Seen {
                charged,
                len: meta.len(),
                extent: dump.extent,
                shown: dump.path,
                path,
                cwd,
            })
        })
        .collect();
    (files, readable)
}

enum Verdict {
    /// The whole dumps of regular files, together over [`REFUSABLE`].
    Refuse(Vec<Seen>),
    /// Over [`REFUSABLE`] too, but the command's `||` structure could not be
    /// read, so which dumps are certain to print is not known: advice only.
    Unreadable(Vec<Seen>),
    Advise(Seen),
    Silent(&'static str),
}

fn judge(context: &Context) -> Verdict {
    let (mut all, readable) = seen(context);
    if all.is_empty() {
        return Verdict::Silent("the file is not there to read");
    }
    let total: u64 = all.iter().filter_map(|s| s.charged).sum();
    if total > REFUSABLE {
        all.retain(|s| s.charged.is_some());
        return if readable {
            Verdict::Refuse(all)
        } else {
            Verdict::Unreadable(all)
        };
    }
    match all.into_iter().find(|s| qualifies(s.extent, Some(s.len))) {
        Some(advised) => Verdict::Advise(advised),
        None => Verdict::Silent("the file is small"),
    }
}

/// The files `confirm` refused, for the hook to bind the refuse-once telling
/// to: empty when the command is not refused.
pub fn refused(context: &Context) -> Vec<Refusal> {
    match judge(context) {
        Verdict::Refuse(files) => files
            .into_iter()
            .map(|s| Refusal {
                path: s.path,
                cwd: s.cwd,
            })
            .collect(),
        Verdict::Unreadable(_) | Verdict::Advise(_) | Verdict::Silent(_) => Vec::new(),
    }
}

/// The most files a reason lists by name.
pub const LISTED: usize = 5;

fn kilobytes(len: u64) -> u64 {
    len.div_ceil(1024)
}

fn refusal_reason(files: &[Seen]) -> String {
    if let [only] = files {
        return format!(
            "{} It is {} KB, over the 8 KB past which a whole dump is refused.",
            pour_reason(&only.shown),
            kilobytes(only.len)
        );
    }
    let total: u64 = files.iter().map(|s| kilobytes(s.len)).sum();
    let mut listed: Vec<String> = files
        .iter()
        .take(LISTED)
        .map(|s| format!("`{}` ({} KB)", s.shown, kilobytes(s.len)))
        .collect();
    if files.len() > LISTED {
        listed.push(format!("and {} more", files.len() - LISTED));
    }
    format!(
        "This command pours whole files into the tool result: {}. Together {total} KB, over the \
         8 KB past which whole dumps are refused: no line numbers, no size cap, and every \
         later turn of this session carries them again.",
        listed.join(", "),
    )
}

/// What to do instead, for the dumped `files`, starting with the largest:
/// code LSP answers for is found with LSP, not with the grep this rule's
/// remedy names for any other file. `None` when `files` is empty.
fn remedy_for(files: &[Seen]) -> Option<String> {
    let largest = largest(files)?;
    let several = files.len() > 1;
    let lsp = crate::lsp::is_code(&largest.shown) && crate::lsp::serves(&largest.path).is_ok();
    let shown = &largest.shown;
    Some(match (lsp, several) {
        (true, false) => {
            "Ask LSP first (`documentSymbol`, `goToDefinition`), then open the file with \
             the Read tool, with `offset`/`limit` around that line. `cat` is fine inside \
             a pipe or a heredoc."
                .to_string()
        }
        (true, true) => format!(
            "Ask LSP first (`documentSymbol`, `goToDefinition`), then read the files \
             separately with the Read tool, starting with the largest, `{shown}`, with \
             `offset`/`limit` around the lines you need. `cat` is fine inside a pipe or a \
             heredoc."
        ),
        (false, false) => pour_remedy(shown),
        (false, true) => format!(
            "Read the files separately with the Read tool, starting with the largest, \
             `{shown}`, with `offset`/`limit` around the part you need — \
             `grep -n '<anchor>' {shown} | head` finds the line first. `cat` is fine inside \
             a pipe or a heredoc."
        ),
    })
}

fn largest(files: &[Seen]) -> Option<&Seen> {
    files.iter().max_by_key(|s| s.len)
}

fn confirm(context: &Context, _finding: &Finding) -> Confirmed {
    match judge(context) {
        Verdict::Silent(why) => Confirmed::No(why),
        Verdict::Advise(file) => Confirmed::YesSaying {
            floor: None,
            ceiling: Some(Stance::Advise),
            reason: pour_reason(&file.shown),
            remedy: remedy_for(std::slice::from_ref(&file)),
            excerpt: file.shown,
        },
        Verdict::Refuse(files) => {
            let Some(remedy) = remedy_for(&files) else {
                return Confirmed::No("no whole dump is charged");
            };
            Confirmed::YesSaying {
                floor: None,
                ceiling: None,
                reason: refusal_reason(&files),
                remedy: Some(remedy),
                excerpt: excerpt_of(&files),
            }
        }
        Verdict::Unreadable(files) => {
            let Some(remedy) = remedy_for(&files) else {
                return Confirmed::No("no whole dump is charged");
            };
            Confirmed::YesSaying {
                floor: None,
                ceiling: Some(Stance::Advise),
                reason: format!(
                    "{} The command's `||` structure could not be read, so this is advice, \
                     not a refusal.",
                    refusal_reason(&files)
                ),
                remedy: Some(remedy),
                excerpt: excerpt_of(&files),
            }
        }
    }
}

fn excerpt_of(files: &[Seen]) -> String {
    files
        .iter()
        .map(|s| s.shown.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::lex;

    fn fires(command: &str) -> bool {
        examine(&lex(command)).is_some()
    }

    #[test]
    fn a_whole_or_long_dump_fires() {
        assert!(fires("cat app/lib/landscapeDoc.ts"));
        assert!(fires("cd /x && cat CLAUDE.md"));
        assert!(fires("cat -n src/main.rs"));
        assert!(fires("sed -n '1,400p' crates/x/src/lib.rs"));
        assert!(fires("head -200 build.log"));
        assert!(fires("tail -n 300 build.log"));
    }

    #[test]
    fn only_the_right_of_an_or_is_an_alternative() {
        let at = |src: &str, word: &str| src.find(word).unwrap();
        let src = "cat aa missing || cat bb";
        let spans = alternatives(src).unwrap();
        assert!(spans.iter().any(|s| s.contains(&at(src, "cat bb"))));
        assert!(!spans.iter().any(|s| s.contains(&0)));
        let src = "cat a && cat b";
        assert!(alternatives(src).unwrap().is_empty());
    }

    #[test]
    fn a_window_a_pipe_or_an_edit_is_silent() {
        assert!(!fires("sed -n '115,140p' app/x.tsx"));
        assert!(!fires("head -20 build.log"));
        assert!(!fires("tail -5 build.log"));
        assert!(!fires("cat f.rs | grep -n fn"));
        assert!(!fires("cat a.txt > b.txt"));
        assert!(!fires("cat > notes.md <<'EOF'\nhi\nEOF\n"));
        assert!(!fires("sed -i 's/a/b/' f.rs"));
        assert!(!fires("cat $F"));
        assert!(!fires("git log --oneline | head"));
    }
}
