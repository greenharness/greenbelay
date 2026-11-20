//! `push-browser-check` — a push of interface changes whose tree no
//! `browser-check` agent of this session has looked at.
//!
//! ```sh
//! git push -u origin feat/settings-toggle   # app/routes/settings.tsx changed
//! ```
//!
//! The fleet says interface work is done when an agent has looked at the
//! rendered result in a real browser. Nothing bound that look to what was
//! pushed: a change could go out with no check, or with the check of an
//! older tree. A check counts here only when it names the pushed tree's
//! content id (`crate::content_id`), so any later edit makes it stale by
//! construction.
//!
//! ## Where it speaks
//!
//! `examine` fires on the shape of a push, as `push-preview` does. `confirm`
//! resolves the push (`greenbelay_guard::push_target`) and speaks only when
//! all hold: the repository has a user interface and the pushed branch
//! changes a rendered file (`crate::preview`, the same detection as
//! `push-preview`: tests, test-tool configs and comment-only diffs never
//! count); no pass is remembered for the tree; the landed, approved plan does
//! not declare `## Browser check` → `none: <reason>`; and neither this
//! session's transcript nor one of its side transcripts
//! (`<session>/subagents/*.jsonl`, where a fork runs its checks) holds a
//! completed `browser-check` agent whose prompt carries the pushed tree's
//! `<<<TREE repo=… sha=…>>>` block and whose result has a `browser: ` line and
//! a `url: ` line. A report that stops on `unreachable:` is not a check.
//!
//! A pass is remembered in `<journal>/push-browser-check/by-tree/<repo>/<id>.json`,
//! written by the hook alone (`crate::implementation_review::guards_path`).
//! There is no carry-over by patch: a rebase onto a moved base can change
//! what renders, so only the exact tree passes.
//!
//! No question goes to a person: the only push-time way through is the check
//! itself, or the plan's declaration. Where the `browser-check` agent is not
//! installed the rule says so as advice at every stance and never holds the
//! push (ADR-0073); `doctor` lists the gap too. An unreadable push shape is
//! journalled and passes, except under `deny`, where it is held so that
//! `--all` is not the way around the gate.

use crate::agents::{self, Need};
use crate::implementation_review::{self as review, Judged};
use crate::plan_review::{completed_agents_resumable, side_transcripts};
use greenbelay_guard::journal;
use greenbelay_guard::push_target::{self, Kind, Push};
use greenbelay_guard::rules::{
    Confirmed, Context, Evidence, Examine, Finding, Rule, Stance, Trend,
};
use greenbelay_guard::shell::{Parsed, Simple};
use std::path::{Path, PathBuf};

pub const RULE_ID: &str = "push-browser-check";

/// The agent type whose completed run is the evidence.
const AGENT: &str = "browser-check";

/// The browsers a report may name in its `browser: ` line.
const BROWSERS: [&str; 3] = ["chrome-devtools", "playwright", "claude-in-chrome"];

const SECTION: &str = "Browser check";
const DECLARATION_PREFIX: &str = "none:";

pub const RULE: Rule = Rule {
    id: RULE_ID,
    prevents: "A push of interface changes whose tree no browser check of this session has seen",
    irreversible: false,
    default_stance: Stance::Advise,
    max_stance: Stance::Deny,
    // Not measurable by the backtester, which never runs `confirm`: the
    // 7-day journal count replaces this figure.
    evidence: Evidence {
        per_1000: 0.0,
        measured: "2026-11-20",
        trend: Trend::Rare,
    },
    examine: Examine::Legacy(examine),
    confirm: Some(confirm),
};

fn examine(parsed: &Parsed) -> Option<Finding> {
    let cmd = push_target::find(parsed)?;
    if cmd.is_dry_run() || cmd.has_short('n') {
        return None;
    }
    // A push to the default branch is a refusal of its own before any
    // browser check could matter.
    if cmd
        .operands()
        .iter()
        .skip(2)
        .any(|w| to_default_branch(&w.text))
    {
        return None;
    }
    Some(Finding {
        reason: "This push may change the interface without a browser check of its tree."
            .to_string(),
        remedy: concat!(
            "Launch the `browser-check` agent on the dev server serving this commit, with the line `",
            greenbelay_guard::bin!(),
            " tree-sha --block -C <repo> <rev>` prints in its brief, then push again. \
             A UI this repository cannot serve headless is declared in its landed plan \
             (`## Browser check`, first line `none: <reason>`)."
        )
        .to_string(),
        span: cmd.at..cmd.end,
    })
}

fn to_default_branch(refspec: &str) -> bool {
    let dst = refspec.rsplit(':').next().unwrap_or(refspec);
    let dst = dst.strip_prefix('+').unwrap_or(dst);
    let dst = dst.strip_prefix("refs/heads/").unwrap_or(dst);
    matches!(dst, "main" | "master")
}

fn confirm(context: &Context, _finding: &Finding) -> Confirmed {
    let Some(cmd) = push_target::find(context.parsed) else {
        return Confirmed::No("the push could not be read a second time");
    };
    let stance = greenbelay_guard::stance::resolve(&RULE);
    match judge_push(context, cmd, stance) {
        Judged::Decline(why) => Confirmed::No(why),
        // Every case carries its own text: the finding's remedy is replaced.
        Judged::Say { reason, excerpt } | Judged::Profiles { reason, excerpt } => {
            Confirmed::YesSaying {
                remedy: Some(String::new()),
                floor: None,
                ceiling: None,
                reason,
                excerpt,
            }
        }
        // The agent cannot be launched here: advice, never a hold.
        Judged::Gap { reason, excerpt } => Confirmed::YesSaying {
            remedy: Some(String::new()),
            floor: None,
            ceiling: Some(Stance::Advise),
            reason,
            excerpt,
        },
    }
}

/// What this session's agents showed about one tree.
#[derive(Debug, PartialEq, Eq)]
enum Standing {
    /// A completed check of this tree reached a page.
    Bound,
    /// A completed check of this tree reached no page.
    Unreachable,
    /// A check of this session that reached a page read another tree.
    Older(String),
    /// No completed check in this session carried this tree's TREE line.
    Missing,
}

/// Judge the push in `cmd`.
pub fn judge_push(context: &Context, cmd: &Simple, stance: Stance) -> Judged {
    let (repo, targets) = match push_target::resolve_for(context, cmd) {
        Push::Resolved { repo, targets } => (repo, targets),
        Push::DryRun => return Judged::Decline("dry-run"),
        Push::DeleteOnly => return Judged::Decline("delete-only"),
        Push::Refused(_) => return Judged::Decline("land-refused"),
        Push::Unresolvable { repo, shape } => {
            return if stance == Stance::Deny && repo.as_deref().is_some_and(crate::preview::gated) {
                unknown_shape(&format!("the push cannot be read: {shape}"))
            } else {
                Judged::Decline("unresolvable")
            };
        }
    };
    if !crate::preview::gated(&repo) {
        return Judged::Decline("no-ui-repository");
    }
    let mut judged = Judged::Decline("no-interface-change");
    for target in targets.iter().filter(|t| t.kind == Kind::Branch) {
        match crate::preview::carries_ui(&repo, target) {
            None => {
                if stance == Stance::Deny {
                    return unknown_shape("the published files cannot be listed");
                }
                judged = Judged::Decline("files-unlisted");
            }
            Some(false) => {}
            Some(true) => match judge_target(context, &repo, target) {
                Judged::Say { reason, excerpt } => return Judged::Say { reason, excerpt },
                Judged::Gap { reason, excerpt } => return Judged::Gap { reason, excerpt },
                other => judged = other,
            },
        }
    }
    judged
}

/// One branch target that carries an interface change.
fn judge_target(context: &Context, repo: &Path, target: &push_target::Target) -> Judged {
    let pushed = target.src.as_str();
    let tree = match crate::content_id::content_id(repo, pushed) {
        Ok(tree) => tree,
        Err(why) => return unknown(None, &why),
    };
    let Some(name) = review::repo_name(repo) else {
        return unknown(Some(&tree), "the repository has no name");
    };
    if remembered(&name, &tree) {
        return Judged::Decline("remembered");
    }
    if let Some(detail) = crate::preview::plan_declares(repo, pushed, SECTION, DECLARATION_PREFIX) {
        journal::record(&journal::Entry {
            rule: RULE_ID,
            stance: "-",
            outcome: "declared",
            session: "-",
            repo: &name,
            mode: "-",
            excerpt: &detail,
        });
        return Judged::Decline("declared");
    }
    let standing = match context.transcript {
        Some(transcript) => standing_in(transcript, &name, &tree),
        None => Err("the payload carried no transcript_path".to_string()),
    };
    let standing = match standing {
        Ok(standing) => standing,
        Err(why) => {
            if !installed(context, repo) {
                return gap(&tree);
            }
            return unknown(Some(&tree), &why);
        }
    };
    // A bound check proves the agent ran; any other standing needs it here.
    if standing != Standing::Bound && !installed(context, repo) {
        return gap(&tree);
    }
    let (cause, check) = match standing {
        Standing::Bound => {
            // The pass stands on the transcript either way; a store that
            // cannot be written is journalled so that the next session's
            // repeat check has a cause on record.
            if let Err(why) = remember(&name, &tree) {
                journal::record(&journal::Entry {
                    rule: RULE_ID,
                    stance: "-",
                    outcome: "remember-failed",
                    session: "-",
                    repo: &name,
                    mode: "-",
                    excerpt: &format!("tree={} {why}", short(&tree)),
                });
            }
            return Judged::Decline("bound");
        }
        Standing::Unreachable => (
            format!("the browser check of {} reached no page", short(&tree)),
            "unreachable",
        ),
        Standing::Older(old) => (
            format!(
                "a browser check of this session read tree {}, not {}",
                short(&old),
                short(&tree)
            ),
            "stale",
        ),
        // No check at all, or one whose brief carried no TREE line of this
        // tree: the transcript cannot tell them apart, so the text says what
        // it can.
        Standing::Missing => (
            "no browser check in this session carried this tree's TREE line".to_string(),
            "missing",
        ),
    };
    Judged::Say {
        reason: say(&cause, repo, &name, pushed, &tree),
        excerpt: format!("tree={} check={check}", short(&tree)),
    }
}

/// The advice for a push whose tree has no browser check: the cause, then
/// the TREE block alone on its own line.
fn say(cause: &str, repo: &Path, name: &str, pushed: &str, tree: &str) -> String {
    format!(
        concat!(
            "This push changes the interface and {cause}. Launch `browser-check` on the dev \
             server serving this commit, with this line, on a line of its own, in its brief:\n",
            "{block}\n",
            "then push again. (`",
            greenbelay_guard::bin!(),
            " tree-sha --block -C {path} {rev}` prints the line.) A UI this repository \
             cannot serve headless is declared in its landed, approved plan: a \
             `## Browser check` section whose first line is `none: <reason>`."
        ),
        cause = cause,
        block = review::block(name, tree),
        path = shell_word(&repo.to_string_lossy()),
        rev = shell_word(pushed),
    )
}

/// A word the agent's zsh reads back as written: `~` and a leading `=` are
/// expansions there (a branch may start with `=`), so a word holding either
/// is quoted.
fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+@:".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// The push's shape or its files cannot be read: the way through is a shape
/// the rule can read.
fn unknown_shape(why: &str) -> Judged {
    Judged::Say {
        reason: format!(
            "whether this push was browser-checked cannot be told: {why}. Push one branch by \
             name (`git push origin <branch>`), then push again."
        ),
        excerpt: "tree=- check=unknown".to_string(),
    }
}

/// The push names one branch, but what this session checked cannot be read:
/// the way through is a check of this tree.
fn unknown(tree: Option<&str>, why: &str) -> Judged {
    Judged::Say {
        reason: format!(
            concat!(
                "whether this push was browser-checked cannot be told: {why}. Launch \
                 `browser-check` with the line `",
                greenbelay_guard::bin!(),
                " tree-sha --block -C <repo> <rev>` prints in its brief, then push again."
            ),
            why = why
        ),
        excerpt: format!("tree={} check=unknown", tree.map(short).unwrap_or("-")),
    }
}

/// The agent cannot be launched here. That is said at every stance, as
/// advice and never a hold (the `Gap` ceiling is `advise`), so a machine
/// without the agent hears that its UI pushes go unchecked, and nobody waits
/// for a check nobody can run (ADR-0073). `greenbelay doctor` lists it too.
fn gap(tree: &str) -> Judged {
    Judged::Gap {
        reason: format!(
            "{} So this push goes out without a browser check.",
            agents::gap_line(Need::BrowserCheck)
        ),
        excerpt: format!("tree={} check=gap", short(tree)),
    }
}

/// Whether a source carries the `browser-check` agent, looked up from the
/// repository and from the clause's working directory (a fork's worktree
/// may carry it in its own `.claude/agents/`).
fn installed(context: &Context, repo: &Path) -> bool {
    agents::carried(Need::BrowserCheck, repo) || agents::carried(Need::BrowserCheck, context.cwd)
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(12)]
}

/// What the session's transcript, then its side transcripts, show of
/// `name`'s `tree`. The main transcript is read first and the scan stops at
/// the first bound check. An unreadable main transcript is an error; an
/// unreadable side transcript is skipped, since it is only more evidence.
fn standing_in(transcript: &Path, name: &str, tree: &str) -> Result<Standing, String> {
    let mut unreachable = false;
    let mut older: Option<String> = None;
    let files: Vec<PathBuf> = std::iter::once(transcript.to_path_buf())
        .chain(side_transcripts(transcript))
        .collect();
    for (index, file) in files.iter().enumerate() {
        let checks = match checks_in(file, name) {
            Ok(checks) => checks,
            Err(e) if index == 0 => {
                return Err(format!(
                    "the transcript {} could not be read: {e}",
                    file.display()
                ))
            }
            Err(_) => continue,
        };
        for check in checks {
            match (check.tree == tree, check.reached_a_page) {
                (true, true) => return Ok(Standing::Bound),
                (true, false) => unreachable = true,
                (false, true) => older = Some(check.tree),
                (false, false) => {}
            }
        }
    }
    Ok(if unreachable {
        Standing::Unreachable
    } else {
        older.map_or(Standing::Missing, Standing::Older)
    })
}

/// One completed `browser-check` run bound to a tree of the repository.
struct Check {
    tree: String,
    reached_a_page: bool,
}

fn checks_in(transcript: &Path, name: &str) -> std::io::Result<Vec<Check>> {
    let reader = std::io::BufReader::new(std::fs::File::open(transcript)?);
    Ok(completed_agents_resumable(reader, AGENT)
        .into_iter()
        .filter(|c| c.agent == AGENT)
        .flat_map(|c| {
            let reached_a_page = report_reached_a_page(&c.result);
            review::parse_blocks(&c.prompt)
                .into_iter()
                .filter(|(repo, _)| repo == name)
                .map(move |(_, tree)| Check {
                    tree,
                    reached_a_page,
                })
        })
        .collect())
}

/// Whether a report is a check: a `browser: ` line whose first word names
/// one of [`BROWSERS`] (the agent writes `browser: playwright   profile:
/// isolated` on one line) and a `url: ` line whose page is a real one.
/// `unreachable: <url>` alone, a browser named with no page, or a `url:`
/// that ends on `chrome-error:` or `about:blank` (a navigation that failed)
/// is not.
fn report_reached_a_page(report: &str) -> bool {
    let mut browser = false;
    let mut url = false;
    for line in report.lines().map(str::trim) {
        if let Some(named) = line.strip_prefix("browser: ") {
            browser |= named
                .split_whitespace()
                .next()
                .is_some_and(|first| BROWSERS.contains(&first));
        } else if let Some(page) = line.strip_prefix("url: ") {
            let page = page.split_whitespace().next().unwrap_or("");
            url |= !page.is_empty()
                && !page.starts_with("chrome-error:")
                && !page.starts_with("about:blank");
        }
    }
    browser && url
}

/// One remembered pass, as the hook writes and reads it.
#[derive(serde::Serialize, serde::Deserialize)]
struct Pass {
    repo: String,
    tree: String,
    agent: String,
    how: String,
}

fn pass_file(repo: &str, tree: &str) -> Option<PathBuf> {
    if repo.contains('/') || repo.contains("..") || !review::is_sha256(tree) {
        return None;
    }
    Some(
        journal::dir()?
            .join(RULE_ID)
            .join("by-tree")
            .join(repo)
            .join(format!("{tree}.json")),
    )
}

/// Whether a pass is remembered for this repository and tree.
fn remembered(repo: &str, tree: &str) -> bool {
    pass_file(repo, tree)
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| serde_json::from_str::<Pass>(&text).ok())
        .is_some_and(|pass| pass.tree == tree)
}

/// Written only by the hook, from a bound check in the transcript. The
/// error names what could not be written; the caller journals it.
fn remember(repo: &str, tree: &str) -> std::io::Result<()> {
    let file = pass_file(repo, tree).ok_or_else(|| {
        std::io::Error::other("no journal directory, or a repository name the store refuses")
    })?;
    let parent = file
        .parent()
        .ok_or_else(|| std::io::Error::other("the pass file has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    journal::private(parent, 0o700);
    let body = serde_json::to_string(&Pass {
        repo: repo.to_string(),
        tree: tree.to_string(),
        agent: AGENT.to_string(),
        how: "transcript".to_string(),
    })?;
    greenbelay_guard::atomic::write_atomic(&file, &body)
        .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", file.display())))?;
    journal::private(&file, 0o600);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use greenbelay_guard::shell::lex;

    fn fires(command: &str) -> bool {
        examine(&lex(command)).is_some()
    }

    #[test]
    fn a_push_is_examined() {
        assert!(fires("git push -u origin feat/x"));
        assert!(fires("cd ../wt && git push origin HEAD:feat/x"));
        assert!(fires("git -C ../wt push origin feat/x"));
    }

    #[test]
    fn a_dry_run_the_default_branch_and_other_commands_are_not() {
        assert!(!fires("git push --dry-run origin feat/x"));
        assert!(!fires("git push -n origin feat/x"));
        assert!(!fires("git pull"));
        assert!(!fires("echo git push"));
        assert!(!fires("git push origin main"));
        assert!(!fires("git push origin HEAD:refs/heads/master"));
    }

    #[test]
    fn a_report_is_a_check_only_with_a_browser_and_a_url() {
        assert!(report_reached_a_page(
            "browser: playwright\nurl: http://localhost:3000/\nnot checked: none"
        ));
        assert!(!report_reached_a_page(
            "unreachable: http://localhost:3000/"
        ));
        assert!(!report_reached_a_page(
            "browser: playwright\nunreachable: http://localhost:3000/"
        ));
        assert!(!report_reached_a_page("browser: lynx\nurl: http://x/"));
        assert!(!report_reached_a_page("url: http://x/"));
        // The agent's real first two lines carry trailing fields.
        assert!(report_reached_a_page(
            "browser: chrome-devtools   profile: isolated\n\
             url: http://localhost:5173/settings   viewport: 1280x800   theme: light\n\
             answers:\n  - q: 32px"
        ));
        // A navigation that failed leaves the browser on an error page.
        assert!(!report_reached_a_page(
            "browser: chrome-devtools   profile: isolated\nurl: chrome-error://chromewebdata/"
        ));
        assert!(!report_reached_a_page(
            "browser: playwright   profile: isolated\nurl: about:blank   viewport: 1280x800"
        ));
    }

    #[test]
    fn the_advice_carries_the_block_alone_on_a_line_and_asks_nothing() {
        let tree = "ab".repeat(32);
        let text = say(
            "no browser check ran in this session",
            Path::new("/work/app"),
            "app",
            "feat/x",
            &tree,
        );
        let block = review::block("app", &tree);
        assert!(text.lines().any(|l| l == block), "{text}");
        assert!(text.contains("tree-sha --block -C /work/app feat/x"));
        assert!(!text.contains('?'));
    }
}
