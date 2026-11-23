//! What the size kind of `clear-at-boundary` decided, kept apart from the
//! journal (ADR-0083). The journal rotates; the seven-day follow-up of the
//! size boundary needs every handoff request and the clear-relay submission
//! that answered it, so each size decision is also one line in a day file
//! under `<data dir>/size-evidence/`, kept [`KEEP_DAYS`] days.
//!
//! [`report`] reads the day files back, drops repeated lines, and pairs each
//! request with the clear-relay's `submit` line of the same session after
//! it. A session whose clear-relay log is missing is counted apart from one
//! whose log shows no submission: missing evidence is not an unheeded
//! request.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use greenbelay_guard::civil;
use greenbelay_guard::journal;
use serde_json::Value;

/// How long a day file is kept: past the 14 days the follow-up needs.
pub const KEEP_DAYS: u64 = 30;

/// How long after a request a clear-relay submission still answers it.
pub const ANSWER_WINDOW: Duration = Duration::from_secs(2 * 3600);

/// One size decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// The uuid of the tail's last entry at the Stop.
    pub stop_uuid: String,
    /// The context size, in tokens.
    pub size: u64,
    /// `size`, or `command` for a command boundary answered at the size
    /// stance.
    pub kind: &'static str,
    /// The effective stance.
    pub stance: &'static str,
    /// `requested` (a Continue), `advised`, `observed`, `held background`,
    /// `held relais` or `cap`.
    pub outcome: String,
}

fn dir() -> Option<PathBuf> {
    Some(journal::dir()?.join("size-evidence"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The day file a time falls in, `YYYY-MM-DD.jsonl` in UTC.
fn day_file(time_ms: u64) -> String {
    let day = i64::try_from(time_ms / 86_400_000).unwrap_or(0);
    format!("{}.jsonl", civil::iso(day))
}

/// The line one decision is kept as.
pub(crate) fn line(session: &str, evidence: &Evidence, time_ms: u64) -> String {
    serde_json::json!({
        "session": session,
        "stop_uuid": evidence.stop_uuid,
        "time_ms": time_ms,
        "size": evidence.size,
        "kind": evidence.kind,
        "stance": evidence.stance,
        "outcome": evidence.outcome,
    })
    .to_string()
}

/// Append one decision to today's file. A write that fails is lost: the
/// evidence is a record and never changes the decision.
pub fn append(session: &str, evidence: &Evidence) {
    let Some(dir) = dir() else { return };
    let _ = append_in(&dir, session, evidence, now_ms()); // a lost line is a lost record, not a failure of the Stop
}

pub(crate) fn append_in(
    dir: &Path,
    session: &str,
    evidence: &Evidence,
    time_ms: u64,
) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    journal::private(dir, 0o700);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(day_file(time_ms)))?;
    writeln!(file, "{}", line(session, evidence, time_ms))
}

/// Day files older than [`KEEP_DAYS`] are removed.
pub fn sweep() {
    let Some(dir) = dir() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let keep = Duration::from_secs(KEEP_DAYS * 24 * 3600);
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age > keep);
        if old {
            let _ = std::fs::remove_file(entry.path()); // a file already gone is swept
        }
    }
}

/// One kept decision, as read back.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Row {
    pub session: String,
    pub stop_uuid: String,
    pub time_ms: u64,
    pub size: u64,
    pub kind: String,
    pub stance: String,
    pub outcome: String,
}

fn parse_row(text: &str) -> Option<Row> {
    let row: Value = serde_json::from_str(text).ok()?;
    let text_of = |name: &str| row[name].as_str().map(str::to_string);
    Some(Row {
        session: text_of("session")?,
        stop_uuid: text_of("stop_uuid")?,
        time_ms: row["time_ms"].as_u64()?,
        size: row["size"].as_u64()?,
        kind: text_of("kind")?,
        stance: text_of("stance")?,
        outcome: text_of("outcome")?,
    })
}

/// The rows of every day file in `dir` at or after `since_ms`, oldest first,
/// with repeats dropped: a Stop judged twice (the same session, Stop uuid,
/// kind and outcome) counts once, at its first time.
pub fn rows_in(dir: &Path, since_ms: u64) -> Vec<Row> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rows: Vec<Row> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .flat_map(|text| text.lines().filter_map(parse_row).collect::<Vec<_>>())
        .filter(|row| row.time_ms >= since_ms)
        .collect();
    rows.sort_by(|a, b| a.time_ms.cmp(&b.time_ms).then_with(|| a.cmp(b)));
    let mut seen = BTreeSet::new();
    rows.retain(|row| {
        seen.insert((
            row.session.clone(),
            row.stop_uuid.clone(),
            row.kind.clone(),
            row.outcome.clone(),
        ))
    });
    rows
}

/// What happened after one handoff request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// A clear-relay `submit` of the same session followed within
    /// [`ANSWER_WINDOW`].
    Submitted,
    /// The session's clear-relay log exists and shows no such submission.
    Unheeded,
    /// No clear-relay log for the session: the evidence is missing.
    Missing,
}

/// The times, in ms, of the `submit` lines of a clear-relay log; `None` when
/// the log cannot be read.
fn submit_times(log: &Path) -> Option<Vec<u64>> {
    let text = std::fs::read_to_string(log).ok()?;
    Some(
        text.lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|entry| entry["event"].as_str() == Some("submit"))
            .filter_map(|entry| entry["time"].as_u64())
            .collect(),
    )
}

/// The answer to a request, from the clear-relay logs in `relay_dir`.
pub fn answer(relay_dir: &Path, request: &Row) -> Answer {
    let Some(times) = submit_times(&relay_dir.join(format!("{}.jsonl", request.session))) else {
        return Answer::Missing;
    };
    let window = u64::try_from(ANSWER_WINDOW.as_millis()).unwrap_or(u64::MAX);
    let answered = times
        .iter()
        .any(|time| *time >= request.time_ms && *time - request.time_ms <= window);
    if answered {
        Answer::Submitted
    } else {
        Answer::Unheeded
    }
}

/// The counts the follow-up reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Size fires and command boundaries past the size, by outcome.
    pub by_outcome: Vec<(String, String, usize)>,
    pub requests: usize,
    pub submitted: usize,
    pub unheeded: usize,
    pub missing: usize,
}

/// The report over `evidence_dir` and `relay_dir` since `since_ms`.
pub fn report_in(evidence_dir: &Path, relay_dir: &Path, since_ms: u64) -> Report {
    let rows = rows_in(evidence_dir, since_ms);
    let mut report = Report::default();
    for row in &rows {
        match report
            .by_outcome
            .iter_mut()
            .find(|(kind, outcome, _)| *kind == row.kind && *outcome == row.outcome)
        {
            Some((_, _, count)) => *count += 1,
            None => report
                .by_outcome
                .push((row.kind.clone(), row.outcome.clone(), 1)),
        }
        if row.outcome == "requested" {
            report.requests += 1;
            match answer(relay_dir, row) {
                Answer::Submitted => report.submitted += 1,
                Answer::Unheeded => report.unheeded += 1,
                Answer::Missing => report.missing += 1,
            }
        }
    }
    report.by_outcome.sort();
    report
}

/// [`report_in`] over the live directories, for the last `days` days.
pub fn report(days: u64) -> Option<Report> {
    let evidence_dir = dir()?;
    let relay_dir = greenbelay_guard::settings::state_root()?.join("clear-relay");
    let since = now_ms().saturating_sub(days * 86_400_000);
    Some(report_in(&evidence_dir, &relay_dir, since))
}

impl Report {
    /// The report as the follow-up reads it.
    pub fn render(&self, days: u64) -> String {
        let mut out = format!("size evidence, last {days} days\n");
        for (kind, outcome, count) in &self.by_outcome {
            out.push_str(&format!("  {kind} {outcome}: {count}\n"));
        }
        out.push_str(&format!(
            "requests {}: submitted {}, unheeded {}, evidence missing {}\n",
            self.requests, self.submitted, self.unheeded, self.missing
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "greenbelay-size-evidence-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir); // left by a killed run, if any
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn requested(uuid: &str) -> Evidence {
        Evidence {
            stop_uuid: uuid.to_string(),
            size: 450_000,
            kind: "size",
            stance: "deny",
            outcome: "requested".to_string(),
        }
    }

    const DAY: u64 = 86_400_000;

    #[test]
    fn requests_pair_with_submissions_and_missing_logs_count_apart() {
        let root = scratch("pair");
        let evidence = root.join("evidence");
        let relay = root.join("relay");
        std::fs::create_dir_all(&relay).unwrap();
        let t = 20 * DAY;
        append_in(&evidence, "s-heeded", &requested("u1"), t).unwrap();
        // The same Stop judged twice is one request.
        append_in(&evidence, "s-heeded", &requested("u1"), t + 5).unwrap();
        append_in(&evidence, "s-unheeded", &requested("u2"), t).unwrap();
        append_in(&evidence, "s-nolog", &requested("u3"), t).unwrap();
        // A request the next day lands in its own file.
        append_in(&evidence, "s-late", &requested("u4"), t + DAY).unwrap();
        std::fs::write(
            relay.join("s-heeded.jsonl"),
            format!("{{\"event\":\"arm\",\"time\":{t}}}\n{{\"event\":\"submit\",\"named\":null,\"time\":{}}}\n", t + 60_000),
        )
        .unwrap();
        std::fs::write(
            relay.join("s-unheeded.jsonl"),
            format!(
                "{{\"event\":\"submit\",\"named\":null,\"time\":{}}}\n",
                t - 1
            ),
        )
        .unwrap();
        std::fs::write(relay.join("s-late.jsonl"), "").unwrap();
        let report = report_in(&evidence, &relay, 0);
        assert_eq!(report.requests, 4);
        assert_eq!(report.submitted, 1);
        assert_eq!(report.unheeded, 2);
        assert_eq!(report.missing, 1);
        assert_eq!(std::fs::read_dir(&evidence).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_report_survives_a_journal_rotation() {
        // The day files live outside the journal: rotating the journal
        // (moving its files away) leaves every request readable.
        let root = scratch("rotation");
        let evidence = root.join("size-evidence");
        let journal_file = root.join("journal.tsv");
        std::fs::write(&journal_file, "header\nrow\n").unwrap();
        append_in(&evidence, "s", &requested("u"), 3 * DAY).unwrap();
        std::fs::rename(&journal_file, root.join("journal.tsv.1")).unwrap();
        let report = report_in(&evidence, &root.join("relay"), 0);
        assert_eq!((report.requests, report.missing), (1, 1));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rows_before_the_window_are_left_out() {
        let root = scratch("window");
        append_in(&root, "s", &requested("old"), DAY).unwrap();
        append_in(&root, "s", &requested("new"), 9 * DAY).unwrap();
        let rows = rows_in(&root, 2 * DAY);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].stop_uuid, "new");
        let _ = std::fs::remove_dir_all(&root);
    }
}
