//! `land-retry`: did an `amont land` push the commit the guards judged?
//!
//! A land retries once when its pre-push hook fast-forwards HEAD, and the
//! retry pushes a commit nobody judged: the implementation review and the
//! preview looked at the sha before the hook ran. A plain `git push` retry is
//! a new command and is judged again; a land's is not, so this checks after
//! the fact, from the land's own receipt (amont 1.54.0), never from the
//! tracking ref, which git moves on its own from 2.56.
//!
//! PreToolUse recorded the judged sha (`preview::record_before`, every
//! repository). The receipt written after that time for the same branch is
//! the one this call made: the shape the guards accept holds exactly one land.
//!
//! - `landed`, `unverified` or `not-observed` with another sha: said, as an
//!   unreviewed push (the last two: the remote accepted it, so possibly);
//! - `refused`, `unreachable`, `interrupted`: nothing was published;
//! - no record, no receipt, or more than one: `Unknown`, journalled as
//!   `retry-check skipped: …`, never a silent pass.
//!
//! It runs on `PostToolUseFailure` too: a land that ends `unverified` or
//! `not-observed` exits non-zero after the remote accepted the push.

use crate::assertions::{Assertion, Claim, Verdict};
use crate::rules::{Context, Evidence, Stance, Trend};
use crate::shell::Parsed;

pub const ASSERTION: Assertion = Assertion {
    id: "land-retry",
    prevents:
        "A land retry pushing a commit that the implementation review and preview never judged",
    irreversible: false,
    default_stance: Stance::Advise,
    evidence: Evidence {
        per_1000: 0.0,
        measured: "2026-10-14",
        trend: Trend::Rare,
    },
    examine,
    verify,
};

fn examine(parsed: &Parsed) -> Option<Claim> {
    // The land that publishes is the one to judge: a lone `--wait` beside it
    // (`… --wait r && amont land`) must not take its place. A command whose
    // only land is a wait claims the wait.
    let lands: Vec<_> = parsed
        .judgeable()
        .filter(|c| crate::land::is_land(c))
        .collect();
    let cmd = lands
        .iter()
        .find(|c| !crate::land::is_wait_only(c))
        .or(lands.first())
        .copied()?;
    if cmd.is_dry_run() {
        return None;
    }
    Some(Claim {
        what: "landed what was judged".to_string(),
        span: cmd.at..cmd.end,
    })
}

fn verify(context: &Context, claim: &Claim) -> Verdict {
    let waited = context
        .parsed
        .judgeable()
        .find(|c| c.at == claim.span.start)
        .filter(|c| crate::land::is_wait_only(c));
    if let Some(cmd) = waited {
        return verify_wait(context, cmd);
    }
    let Some(rec) = crate::push_record::land_judged(context.tool_use_id) else {
        return Verdict::Unknown("retry-check skipped: no judgement was recorded before the land");
    };
    let found = crate::land::receipts_since(&rec.receipt_dir, &rec.branch, rec.since);
    let receipt = match found.as_slice() {
        [one] => one,
        [] => return Verdict::Unknown("retry-check skipped: the land left no receipt"),
        _ => return Verdict::Unknown("retry-check skipped: more than one receipt for this land"),
    };
    judge_receipt(&rec.judged, receipt)
}

/// Pure: the receipt this call's land left, against the sha the guards
/// judged before it ran. A detached land still running is judged on the sha
/// it was launched on; any other receipt on what it pushed.
fn judge_receipt(judged: &str, receipt: &crate::land::Receipt) -> Verdict {
    if let Some(broken) = launched_on_another(judged, &receipt.status, receipt.judged.as_deref()) {
        return broken;
    }
    judge(judged, &receipt.status, &receipt.pushed)
}

/// A lone `land --wait <receipt>` ends a detached land: the receipt it names
/// says what was pushed and what the land's own worker judged.
fn verify_wait(context: &Context, cmd: &crate::shell::Simple) -> Verdict {
    let Some(path) = crate::land::wait_path(cmd) else {
        return Verdict::Unknown("retry-check skipped: the receipt path could not be read");
    };
    let dir = match context.cwd_walk(cmd.at) {
        crate::rules::Walk::Walked(dir) => dir,
        crate::rules::Walk::VariableCd { .. } => {
            return Verdict::Unknown("retry-check skipped: the directory is a shell variable")
        }
    };
    let Ok(text) = std::fs::read_to_string(dir.join(path)) else {
        return Verdict::Unknown("retry-check skipped: the receipt could not be read");
    };
    let Some(receipt) = crate::land::read_receipt(&text) else {
        return Verdict::Unknown("retry-check skipped: the receipt is not one this version reads");
    };
    let Some(judged) = receipt.judged.as_deref() else {
        return Verdict::Unknown("retry-check skipped: the receipt records no judged commit");
    };
    judge(judged, &receipt.status, &receipt.pushed)
}

/// Pure: a detached land's worker pushes only the sha its launcher recorded,
/// so a running receipt whose recorded sha is not the judged one will push a
/// commit the review never saw. `None` when there is nothing to object to.
fn launched_on_another(judged: &str, status: &str, launched: Option<&str>) -> Option<Verdict> {
    let launched = launched.filter(|l| status == "running" && *l != judged)?;
    let short = |s: &str| s.chars().take(7).collect::<String>();
    Some(Verdict::Broken {
        reason: format!(
            "This detached land will push {}, but the guards judged {}: its worker pushes \
             only the commit it was launched on, which no review or preview has seen.",
            short(launched),
            short(judged)
        ),
        remedy: "Stop the worker before it pushes (kill the pid in its receipt), then land the \
                 judged commit, or review the launched one."
            .to_string(),
    })
}

/// Pure: the judged sha against what the receipt says was pushed.
fn judge(judged: &str, status: &str, pushed: &str) -> Verdict {
    let published = match status {
        "landed" => true,
        "unverified" | "not-observed" => false,
        // A detached land's worker has not finished: its final receipt is
        // judged when it lands, not on a guess.
        "running" => return Verdict::Unknown("retry-check deferred: detached land still running"),
        // Nothing was published.
        "refused" | "unreachable" | "interrupted" => return Verdict::Held,
        // A status a newer amont added: not a pass on a guess.
        _ => {
            return Verdict::Unknown(
                "retry-check skipped: a receipt status this version does not know",
            )
        }
    };
    if pushed == judged {
        return Verdict::Held;
    }
    let short = |s: &str| s.chars().take(7).collect::<String>();
    let how = if published {
        "pushed"
    } else {
        "possibly pushed"
    };
    Verdict::Broken {
        reason: format!(
            "This land {how} {} after its pre-push hook moved HEAD, but the guards judged {}: \
             the commit on the remote is one no review or preview has seen.",
            short(pushed),
            short(judged)
        ),
        remedy:
            "Review the commit that was pushed (worktree-task F4b on the branch as it is now), \
                 and do not merge the pull request until that review is settled."
                .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::lex;

    #[test]
    fn a_land_is_claimed_and_a_dry_run_is_not() {
        assert!(examine(&lex("amont land --json")).is_some());
        assert!(examine(&lex("cd x && amont land")).is_some());
        assert!(examine(&lex("amont land --dry-run --json")).is_none());
        assert!(examine(&lex("git push origin feat")).is_none());
        assert!(examine(&lex("greengate land --wait /r/receipt.json")).is_some());
        assert!(examine(&lex("greengate land --detach --json")).is_some());
    }

    #[test]
    fn the_judged_sha_landing_holds() {
        assert!(matches!(judge("aaa", "landed", "aaa"), Verdict::Held));
    }

    #[test]
    fn a_retried_sha_is_said() {
        assert!(matches!(
            judge("aaa", "landed", "bbb"),
            Verdict::Broken { .. }
        ));
        match judge("aaa", "not-observed", "bbb") {
            Verdict::Broken { reason, .. } => assert!(reason.contains("possibly")),
            _ => panic!("an accepted push of another sha is said"),
        }
    }

    #[test]
    fn an_unknown_status_is_skipped_not_passed() {
        assert!(matches!(
            judge("aaa", "landed-twice", "bbb"),
            Verdict::Unknown(_)
        ));
    }

    #[test]
    fn a_running_receipt_defers_and_a_landed_one_is_judged() {
        assert!(matches!(
            judge("aaa", "running", "bbb"),
            Verdict::Unknown("retry-check deferred: detached land still running")
        ));
        assert!(matches!(
            judge("aaa", "landed", "bbb"),
            Verdict::Broken { .. }
        ));
    }

    #[test]
    fn nothing_published_is_not_said() {
        for status in ["refused", "unreachable", "interrupted"] {
            assert!(
                matches!(judge("aaa", status, "bbb"), Verdict::Held),
                "{status}"
            );
        }
    }

    fn waited_verdict(receipt_json: &str) -> Verdict {
        // One directory per call: tests run in parallel threads of one process.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "greenbelay-land-retry-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("1.json");
        std::fs::write(&path, receipt_json).unwrap();
        // A relative path read from the call's directory: a Windows path's
        // backslashes are shell escapes, and the join is what verify_wait does.
        let command = "greengate land --wait 1.json".to_string();
        let parsed = lex(&command);
        let claim = examine(&parsed).expect("a lone wait is claimed");
        let context = Context {
            platform: None,
            cwd: &dir,
            parsed: &parsed,
            background: false,
            timeout_ms: None,
            tool_use_id: "",
            transcript: None,
            agent_transcript: None,
            command: &command,
            output: None,
            session: "",
            failed: false,
        };
        let verdict = verify(&context, &claim);
        let _ = std::fs::remove_dir_all(&dir); // cleanup of a scratch dir: the verdict is already taken
        verdict
    }

    #[test]
    fn a_detached_land_launched_on_an_unjudged_commit_is_broken() {
        assert!(matches!(
            launched_on_another("aaa", "running", Some("bbb")),
            Some(Verdict::Broken { .. })
        ));
        assert!(launched_on_another("aaa", "running", Some("aaa")).is_none());
        assert!(launched_on_another("aaa", "running", None).is_none());
        // Only a running receipt is judged here; a final one goes to judge.
        assert!(launched_on_another("aaa", "landed", Some("bbb")).is_none());
    }

    #[test]
    fn a_running_receipt_is_judged_on_the_sha_it_was_launched_on() {
        let running = |launched: &str| {
            crate::land::read_receipt(&format!(
                r#"{{"started":5,"branch":"feat","pushed":"","status":"running","judged":"{launched}"}}"#
            ))
            .expect("a running receipt reads")
        };
        assert!(matches!(
            judge_receipt("aaa", &running("bbb")),
            Verdict::Broken { .. }
        ));
        assert!(matches!(
            judge_receipt("aaa", &running("aaa")),
            Verdict::Unknown("retry-check deferred: detached land still running")
        ));
    }

    #[test]
    fn a_wait_beside_a_land_does_not_take_its_place() {
        let parsed = lex("greengate land --wait /r/x.json && amont land --json");
        let claim = examine(&parsed).expect("the land is claimed");
        let claimed = parsed
            .judgeable()
            .find(|c| c.at == claim.span.start)
            .expect("a clause");
        assert!(!crate::land::is_wait_only(claimed));
        let lone = lex("greengate land --wait /r/x.json");
        let claim = examine(&lone).expect("a lone wait is claimed");
        let claimed = lone.judgeable().find(|c| c.at == claim.span.start).unwrap();
        assert!(crate::land::is_wait_only(claimed));
    }

    #[test]
    fn a_waited_receipt_is_judged_by_its_own_judged_and_pushed() {
        let receipt = |status: &str, pushed: &str| {
            format!(
                r#"{{"started":5,"branch":"feat","pushed":"{pushed}","status":"{status}","judged":"aaa"}}"#
            )
        };
        assert!(matches!(
            waited_verdict(&receipt("landed", "bbb")),
            Verdict::Broken { .. }
        ));
        assert!(matches!(
            waited_verdict(&receipt("landed", "aaa")),
            Verdict::Held
        ));
        assert!(matches!(
            waited_verdict(&receipt("running", "aaa")),
            Verdict::Unknown("retry-check deferred: detached land still running")
        ));
    }
}
