//! What the size kind of `clear-at-boundary` decided, kept apart from the
//! journal (ADR-0083). The journal rotates; the seven-day follow-up of the
//! size boundary needs every handoff request and the clear-relay submission
//! that answered it, so each size decision is also one line in a day file
//! under `<data dir>/size-evidence/`, kept [`KEEP_DAYS`] days.
//!
//! [`report_in`] reads the day files back, drops repeated lines, and pairs
//! each request with the clear-relay's `submit` line of the same session
//! after it. The relay writes a session's log only once it arms, so a session
//! with no log in a readable relay directory is an unheeded request; only an
//! unreadable relay directory, or a log that cannot be read, is missing
//! evidence.
//!
//! The clear-relay log is the plugin's (`plugin/clear-relay/hooks/register.ts`,
//! `log`): `<stateDir>/clear-relay/<Claude session id>.jsonl`, one JSON object
//! per line, read here for `event` (`"submit"`) and `time` (ms since the
//! epoch). [`relay_dir`] is the plugin's default `stateDir`, the one `greenbelay
//! install` writes; `size-evidence --relay-dir` reads another.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use greenbelay_guard::civil;
use greenbelay_guard::journal;
use greenbelay_guard::rules::Stance;
use serde_json::Value;

/// How long a day file is kept: past the 14 days the follow-up needs.
pub const KEEP_DAYS: u64 = 30;

/// How long after a request a clear-relay submission still answers it.
pub const ANSWER_WINDOW: Duration = Duration::from_secs(2 * 3600);

/// Which judgement a decision came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// The size kind itself.
    Size,
    /// A command boundary past the size.
    Command,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Size => "size",
            Kind::Command => "command",
        }
    }

    /// The kind a day file names; `None` for a word this version never wrote.
    pub fn parse(text: &str) -> Option<Kind> {
        [Kind::Size, Kind::Command]
            .into_iter()
            .find(|kind| kind.as_str() == text)
    }
}

/// What one decision past the size did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    /// A Continue asked for the handoff.
    Requested,
    /// A note told the person.
    Advised,
    /// Journalled only.
    Observed,
    /// A background task was running.
    HeldBackground,
    /// A relais run was in flight.
    HeldRelais,
    /// The session's fires were spent.
    Cap,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Requested => "requested",
            Outcome::Advised => "advised",
            Outcome::Observed => "observed",
            Outcome::HeldBackground => "held background",
            Outcome::HeldRelais => "held relais",
            Outcome::Cap => "cap",
        }
    }

    /// The outcome a day file names; `None` for a word this version never
    /// wrote.
    pub fn parse(text: &str) -> Option<Outcome> {
        [
            Outcome::Requested,
            Outcome::Advised,
            Outcome::Observed,
            Outcome::HeldBackground,
            Outcome::HeldRelais,
            Outcome::Cap,
        ]
        .into_iter()
        .find(|outcome| outcome.as_str() == text)
    }

    /// What a fire at `stance` did.
    pub fn of_fire(stance: Stance) -> Outcome {
        match stance {
            Stance::Deny => Outcome::Requested,
            Stance::Advise => Outcome::Advised,
            Stance::Observe => Outcome::Observed,
        }
    }
}

/// One size decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// The uuid of the tail's last entry at the Stop.
    pub stop_uuid: String,
    /// The context size, in tokens.
    pub size: u64,
    pub kind: Kind,
    /// The effective stance.
    pub stance: Stance,
    pub outcome: Outcome,
}

/// The live evidence directory, `<data dir>/size-evidence`.
pub fn dir() -> Option<PathBuf> {
    Some(journal::dir()?.join("size-evidence"))
}

/// The clear-relay plugin's log directory at its default `stateDir`.
pub fn relay_dir() -> Option<PathBuf> {
    Some(
        greenbelay_guard::settings::state_root()?
            .join(greenbelay_guard::names::DATA_DIR)
            .join("clear-relay"),
    )
}

/// The time now, in ms since the epoch.
pub fn now_ms() -> u64 {
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
        "kind": evidence.kind.as_str(),
        "stance": evidence.stance.as_str(),
        "outcome": evidence.outcome.as_str(),
    })
    .to_string()
}

/// Append one decision to today's file. The caller journals a failure; the
/// evidence is a record and never changes the decision.
pub fn append(session: &str, evidence: &Evidence) -> std::io::Result<()> {
    let dir = dir().ok_or_else(|| std::io::Error::other("no data directory"))?;
    append_in(&dir, session, evidence, now_ms())
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
    pub kind: Kind,
    pub stance: Stance,
    pub outcome: Outcome,
}

fn parse_row(text: &str) -> Option<Row> {
    let row: Value = serde_json::from_str(text).ok()?;
    let text_of = |name: &str| row[name].as_str().map(str::to_string);
    Some(Row {
        session: text_of("session")?,
        stop_uuid: text_of("stop_uuid")?,
        time_ms: row["time_ms"].as_u64()?,
        size: row["size"].as_u64()?,
        kind: Kind::parse(row["kind"].as_str()?)?,
        stance: Stance::parse(row["stance"].as_str()?)?,
        outcome: Outcome::parse(row["outcome"].as_str()?)?,
    })
}

/// What [`rows_in`] read back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadBack {
    pub rows: Vec<Row>,
    /// Day files, or the directory itself, that could not be read.
    pub unreadable_days: usize,
    /// Lines that are not a row this version writes (torn, or an unknown
    /// kind, stance or outcome).
    pub unparsed_lines: usize,
}

/// The rows of every day file in `dir` at or after `since_ms`, oldest first,
/// with repeats dropped: a Stop judged twice (the same session, Stop uuid,
/// kind and outcome) counts once, at its first time. A directory that does
/// not exist holds no rows; one that cannot be read is counted unreadable.
pub fn rows_in(dir: &Path, since_ms: u64) -> ReadBack {
    let mut read = ReadBack::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return read,
        Err(_) => {
            read.unreadable_days += 1;
            return read;
        }
    };
    for entry in entries.flatten() {
        if entry.path().extension().is_none_or(|ext| ext != "jsonl") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            read.unreadable_days += 1;
            continue;
        };
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            match parse_row(line) {
                Some(row) if row.time_ms >= since_ms => read.rows.push(row),
                Some(_) => {}
                None => read.unparsed_lines += 1,
            }
        }
    }
    read.rows
        .sort_by(|a, b| a.time_ms.cmp(&b.time_ms).then_with(|| a.cmp(b)));
    let mut seen = BTreeSet::new();
    read.rows.retain(|row| {
        seen.insert((
            row.session.clone(),
            row.stop_uuid.clone(),
            row.kind,
            row.outcome,
        ))
    });
    read
}

/// What happened after one handoff request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// A clear-relay `submit` of the same session followed within
    /// [`ANSWER_WINDOW`].
    Submitted,
    /// No such submission: the session's log shows none, or the relay never
    /// armed and wrote no log.
    Unheeded,
    /// The relay directory or the session's log cannot be read.
    Missing,
}

/// The times, in ms, of the `submit` lines of a clear-relay log: empty when
/// there is no log, `None` when it cannot be read.
fn submit_times(log: &Path) -> Option<Vec<u64>> {
    let text = match std::fs::read_to_string(log) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(Vec::new()),
        Err(_) => return None,
    };
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
    if !relay_dir.is_dir() {
        return Answer::Missing;
    }
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
    /// The relay directory the requests were paired against.
    pub relay_dir: PathBuf,
    /// Size fires and command boundaries past the size, by kind and outcome.
    pub by_outcome: BTreeMap<(Kind, Outcome), usize>,
    pub requests: usize,
    pub submitted: usize,
    pub unheeded: usize,
    pub missing: usize,
    /// Day files that could not be read.
    pub unreadable_days: usize,
    /// Lines of the day files that are not a row this version writes.
    pub unparsed_lines: usize,
}

/// The report over `evidence_dir` and `relay_dir` since `since_ms`.
pub fn report_in(evidence_dir: &Path, relay_dir: &Path, since_ms: u64) -> Report {
    let read = rows_in(evidence_dir, since_ms);
    let mut report = Report {
        relay_dir: relay_dir.to_path_buf(),
        unreadable_days: read.unreadable_days,
        unparsed_lines: read.unparsed_lines,
        ..Report::default()
    };
    for row in &read.rows {
        *report
            .by_outcome
            .entry((row.kind, row.outcome))
            .or_default() += 1;
        if row.outcome == Outcome::Requested {
            report.requests += 1;
            match answer(relay_dir, row) {
                Answer::Submitted => report.submitted += 1,
                Answer::Unheeded => report.unheeded += 1,
                Answer::Missing => report.missing += 1,
            }
        }
    }
    report
}

/// The start of a window of `days` days ending at `now_ms`.
pub fn since(now_ms: u64, days: u64) -> u64 {
    now_ms.saturating_sub(days.saturating_mul(86_400_000))
}

impl Report {
    /// The report as the follow-up reads it: the answer line first.
    pub fn render(&self, days: u64) -> String {
        let mut lines = vec![
            format!(
                "requests {}: submitted {}, unheeded {}, evidence missing {}",
                self.requests, self.submitted, self.unheeded, self.missing
            ),
            format!("window: last {days} days"),
        ];
        lines.extend(self.by_outcome.iter().map(|((kind, outcome), count)| {
            format!("  {} {}: {count}", kind.as_str(), outcome.as_str())
        }));
        lines.push(format!("relay logs read from {}", self.relay_dir.display()));
        if self.unreadable_days > 0 {
            lines.push(format!("unreadable day files: {}", self.unreadable_days));
        }
        if self.unparsed_lines > 0 {
            lines.push(format!("unparsed lines: {}", self.unparsed_lines));
        }
        lines.iter().map(|line| format!("{line}\n")).collect()
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
            kind: Kind::Size,
            stance: Stance::Deny,
            outcome: Outcome::Requested,
        }
    }

    const DAY: u64 = 86_400_000;

    #[test]
    fn requests_pair_with_submissions_and_a_session_without_a_log_is_unheeded() {
        let root = scratch("pair");
        let evidence = root.join("evidence");
        let relay = root.join("relay");
        std::fs::create_dir_all(&relay).unwrap();
        let request_ms = 20 * DAY;
        append_in(&evidence, "s-heeded", &requested("u1"), request_ms).unwrap();
        // The same Stop judged twice is one request.
        append_in(&evidence, "s-heeded", &requested("u1"), request_ms + 5).unwrap();
        append_in(&evidence, "s-unheeded", &requested("u2"), request_ms).unwrap();
        append_in(&evidence, "s-nolog", &requested("u3"), request_ms).unwrap();
        // A request the next day lands in its own file.
        append_in(&evidence, "s-late", &requested("u4"), request_ms + DAY).unwrap();
        std::fs::write(
            relay.join("s-heeded.jsonl"),
            format!("{{\"event\":\"arm\",\"time\":{request_ms}}}\n{{\"event\":\"submit\",\"named\":null,\"time\":{}}}\n", request_ms + 60_000),
        )
        .unwrap();
        std::fs::write(
            relay.join("s-unheeded.jsonl"),
            format!(
                "{{\"event\":\"submit\",\"named\":null,\"time\":{}}}\n",
                request_ms - 1
            ),
        )
        .unwrap();
        std::fs::write(relay.join("s-late.jsonl"), "").unwrap();
        let report = report_in(&evidence, &relay, 0);
        assert_eq!(report.requests, 4);
        assert_eq!(report.submitted, 1);
        // s-unheeded's submit came before the request; s-nolog never armed.
        assert_eq!(report.unheeded, 3);
        assert_eq!(report.missing, 0);
        assert_eq!(std::fs::read_dir(&evidence).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(&root); // a scratch directory; a leftover is harmless
    }

    #[test]
    fn an_unreadable_relay_directory_is_missing_evidence() {
        let root = scratch("norelay");
        let evidence = root.join("evidence");
        append_in(&evidence, "s", &requested("u"), 3 * DAY).unwrap();
        let report = report_in(&evidence, &root.join("absent"), 0);
        assert_eq!(
            (report.requests, report.missing, report.unheeded),
            (1, 1, 0)
        );
        let _ = std::fs::remove_dir_all(&root); // a scratch directory; a leftover is harmless
    }

    #[test]
    fn the_report_survives_a_journal_rotation() {
        // The day files sit under the data_dir directory beside the journal. A
        // journal past its cap rotates on the next record; every request
        // must still read back and still pair with its submission.
        let data_dir = scratch("rotation");
        let evidence = data_dir.join("size-evidence");
        let relay = data_dir.join("relay");
        std::fs::create_dir_all(&relay).unwrap();
        let request_ms = 3 * DAY;
        append_in(&evidence, "s", &requested("u"), request_ms).unwrap();
        std::fs::write(
            relay.join("s.jsonl"),
            format!(
                "{{\"event\":\"submit\",\"named\":null,\"time\":{}}}\n",
                request_ms + 1
            ),
        )
        .unwrap();
        std::fs::write(data_dir.join("journal.log"), vec![b'x'; 300 * 1024]).unwrap();
        journal::record_in(
            &data_dir,
            &journal::Entry {
                rule: "clear-at-boundary",
                stance: "deny",
                outcome: "watched",
                session: "s",
                repo: "-",
                mode: "-",
                excerpt: "rotation probe",
            },
        );
        assert!(
            data_dir.join("journal.log.1").exists(),
            "the journal rotated"
        );
        let report = report_in(&evidence, &relay, 0);
        assert_eq!((report.requests, report.submitted), (1, 1));
        let _ = std::fs::remove_dir_all(&data_dir); // a scratch directory; a leftover is harmless
    }

    #[test]
    fn a_line_this_version_never_wrote_is_counted_apart() {
        let root = scratch("unparsed");
        append_in(&root, "s", &requested("u"), 3 * DAY).unwrap();
        let day = root.join(day_file(3 * DAY));
        let mut text = std::fs::read_to_string(&day).unwrap();
        text.push_str(&line("s", &requested("v"), 3 * DAY).replace("requested", "renamed"));
        text.push_str("\n{\"torn\n");
        std::fs::write(&day, text).unwrap();
        let read = rows_in(&root, 0);
        assert_eq!((read.rows.len(), read.unparsed_lines), (1, 2));
        assert_eq!(read.rows[0].outcome, Outcome::Requested);
        let _ = std::fs::remove_dir_all(&root); // a scratch directory; a leftover is harmless
    }

    #[test]
    fn rows_before_the_window_are_left_out() {
        let root = scratch("window");
        append_in(&root, "s", &requested("old"), DAY).unwrap();
        append_in(&root, "s", &requested("new"), 9 * DAY).unwrap();
        let read = rows_in(&root, 2 * DAY);
        assert_eq!((read.rows.len(), read.unreadable_days), (1, 0));
        assert_eq!(read.rows[0].stop_uuid, "new");
        let _ = std::fs::remove_dir_all(&root); // a scratch directory; a leftover is harmless
    }
}
