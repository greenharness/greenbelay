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
pub fn resolve(cwd: &Path, dump: &Dump) -> Option<PathBuf> {
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
    /// A regular file: a FIFO, a device or a directory is never counted, and
    /// never opened.
    regular: bool,
    exempt: bool,
    /// On the right of `||`: it may print, but is not certain to.
    alternative: bool,
}

impl Seen {
    fn charged(&self) -> bool {
        matches!(self.extent, Extent::Whole) && self.regular && !self.exempt && !self.alternative
    }
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

/// The byte spans of every command on the right of a `||`.
fn alternatives(src: &str) -> Vec<std::ops::Range<usize>> {
    use crate::analysis::ir::{AndOrOp, Cmd};
    fn walk(cmd: &Cmd, out: &mut Vec<std::ops::Range<usize>>) {
        match cmd {
            Cmd::AndOr { first, rest, .. } => {
                walk(first, out);
                for (op, next) in rest {
                    if *op == AndOrOp::Or {
                        out.push(next.span());
                    }
                    walk(next, out);
                }
            }
            Cmd::Seq { items, .. } => items.iter().for_each(|i| walk(&i.cmd, out)),
            Cmd::Pipeline { cmds, .. } => cmds.iter().for_each(|c| walk(c, out)),
            Cmd::If {
                arms, otherwise, ..
            } => {
                for (c, b) in arms {
                    walk(c, out);
                    walk(b, out);
                }
                if let Some(o) = otherwise {
                    walk(o, out);
                }
            }
            Cmd::Case { arms, .. } => arms.iter().for_each(|a| walk(&a.body, out)),
            Cmd::For { body, .. }
            | Cmd::ArithFor { body, .. }
            | Cmd::Group { body, .. }
            | Cmd::Subshell { body, .. }
            | Cmd::FuncDef { body, .. }
            | Cmd::Redirected { body, .. } => walk(body, out),
            Cmd::Loop { cond, body, .. } => {
                walk(cond, out);
                walk(body, out);
            }
            Cmd::Simple(_) | Cmd::Arith { .. } | Cmd::Cond { .. } | Cmd::Unsupported { .. } => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(cmd) = crate::analysis::frontend::parse(src) {
        walk(&cmd, &mut out);
    }
    out
}

/// Every dump of the command whose file is there, looked at with `stat` alone.
fn seen(ctx: &Context) -> Vec<Seen> {
    let alternative_spans = alternatives(ctx.command);
    dumps(ctx.parsed)
        .into_iter()
        .filter_map(|dump| {
            let cwd = ctx.cwd_at(dump.at);
            let path = resolve(&cwd, &dump)?;
            let meta = std::fs::metadata(&path).ok()?;
            Some(Seen {
                alternative: alternative_spans
                    .iter()
                    .any(|span| dump.at < span.end && span.start < dump.end),
                exempt: exempt(&path),
                regular: meta.is_file(),
                len: meta.len(),
                extent: dump.extent,
                shown: dump.path,
                path,
                cwd,
            })
        })
        .collect()
}

enum Verdict {
    /// The whole dumps of regular files, together over [`REFUSABLE`].
    Refuse(Vec<Seen>),
    Advise(Seen),
    Silent(&'static str),
}

fn judge(ctx: &Context) -> Verdict {
    let mut all = seen(ctx);
    if all.is_empty() {
        return Verdict::Silent("the file is not there to read");
    }
    let total: u64 = all.iter().filter(|s| s.charged()).map(|s| s.len).sum();
    if total > REFUSABLE {
        all.retain(Seen::charged);
        return Verdict::Refuse(all);
    }
    match all.into_iter().find(|s| qualifies(s.extent, Some(s.len))) {
        Some(advised) => Verdict::Advise(advised),
        None => Verdict::Silent("the file is small"),
    }
}

/// The files `confirm` refused, for the hook to bind the refuse-once telling
/// to: empty when the command is not refused.
pub fn refused(ctx: &Context) -> Vec<Refusal> {
    match judge(ctx) {
        Verdict::Refuse(files) => files
            .into_iter()
            .map(|s| Refusal {
                path: s.path,
                cwd: s.cwd,
            })
            .collect(),
        Verdict::Advise(_) | Verdict::Silent(_) => Vec::new(),
    }
}

/// The most files a reason lists by name.
const LISTED: usize = 5;

fn kilobytes(len: u64) -> u64 {
    len.div_ceil(1024)
}

fn refusal_reason(files: &[Seen]) -> String {
    let total: u64 = files.iter().map(|s| s.len).sum();
    if let [only] = files {
        return format!(
            "{} It is {} KB, over the 8 KB past which a whole dump is refused.",
            pour_reason(&only.shown),
            kilobytes(only.len)
        );
    }
    let mut listed: Vec<String> = files
        .iter()
        .take(LISTED)
        .map(|s| format!("`{}` ({} KB)", s.shown, kilobytes(s.len)))
        .collect();
    if files.len() > LISTED {
        listed.push(format!("and {} more", files.len() - LISTED));
    }
    format!(
        "This command pours whole files into the tool result: {}. Together {} KB, over the \
         8 KB past which whole dumps are refused: no line numbers, no size cap, and every \
         later turn of this session carries them again.",
        listed.join(", "),
        kilobytes(total)
    )
}

fn remedy_for(shown: &str, path: &Path) -> String {
    // Code LSP answers for is found with LSP, not with the grep this
    // rule's remedy names for any other file.
    if crate::lsp::is_code(shown) && crate::lsp::serves(path).is_ok() {
        "Ask LSP first (`documentSymbol`, `goToDefinition`), then open the file with \
         the Read tool, with `offset`/`limit` around that line. `cat` is fine inside \
         a pipe or a heredoc."
            .to_string()
    } else {
        pour_remedy(shown)
    }
}

fn confirm(ctx: &Context, _finding: &Finding) -> Confirmed {
    match judge(ctx) {
        Verdict::Silent(why) => Confirmed::No(why),
        Verdict::Advise(file) => Confirmed::YesSaying {
            floor: None,
            ceiling: Some(Stance::Advise),
            reason: pour_reason(&file.shown),
            remedy: Some(remedy_for(&file.shown, &file.path)),
            excerpt: file.shown,
        },
        Verdict::Refuse(files) => {
            let largest = files
                .iter()
                .max_by_key(|s| s.len)
                .expect("a refusal holds at least one file");
            Confirmed::YesSaying {
                floor: None,
                ceiling: None,
                reason: refusal_reason(&files),
                remedy: Some(remedy_for(&largest.shown, &largest.path)),
                excerpt: files
                    .iter()
                    .map(|s| s.shown.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            }
        }
    }
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
        let spans = alternatives(src);
        assert!(spans.iter().any(|s| s.contains(&at(src, "cat bb"))));
        assert!(!spans.iter().any(|s| s.contains(&0)));
        let src = "cat a && cat b";
        assert!(alternatives(src).is_empty());
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
