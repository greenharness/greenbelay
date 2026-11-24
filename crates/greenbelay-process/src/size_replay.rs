//! A chronological replay of the size kind of `clear-at-boundary` over
//! recorded transcripts (ADR-0083, plan Verification).
//!
//! Every `Stop` a transcript recorded (a main-chain `system` entry of
//! subtype `stop_hook_summary`) is judged, hook continuations included, with
//! the size state carried from Stop to Stop of the session, and with the
//! production tail limits: the tail is read with [`read_tail_until_at`] up to
//! the Stop's byte offset. The facts a live Stop reads from its payload are
//! rebuilt from the transcript:
//!
//! - permission mode: the last `permissionMode` before the Stop;
//! - question, `WAITING:` and marker: the last main-chain assistant text;
//! - continued turn: an earlier Stop in the same turn;
//! - background work: a launch (`Command running in background with ID:`,
//!   `moved to the background (ID: …)`, an async agent's `agentId:`, a
//!   `Monitor started (task …`) with no
//!   `<task-notification>` naming it yet;
//! - relais in flight: read from the tail, as live.
//!
//! The phase fact needs the repository's git state at the time, which no
//! transcript holds: the replay judges the size kind alone (as at
//! `Working`), so a command boundary past the size is not replayed.
//!
//! A fire counts as held wrongly when a `<task-notification>` arrives after it
//! and before the next person prompt: something was running at the fired
//! Stop that the rebuilt facts missed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use greenbelay_guard::decision::Decision;
use greenbelay_guard::rules::clear_at_boundary::handoff_line;
use greenbelay_guard::rules::Stance;
use serde_json::Value;

use crate::clear_boundary::{
    blocks, context_size, decide, entry_type, is_person_prompt, is_prompt, is_sidechain,
    position_of, read_tail_until_at, Repositories, State, StopView, SIZE_BOUNDARY, TAIL_CAP,
    TAIL_FIRST_CHUNK,
};
use crate::plan_phases::PhaseFact;
use crate::size_evidence::Outcome as SizeOutcome;

/// One fire of the size kind in the replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fire {
    pub session: String,
    /// `YYYY-MM-DD` of the Stop.
    pub day: String,
    pub size: u64,
    /// The Stop was in a turn a hook continued.
    pub continued: bool,
    /// The session's first fire.
    pub first: bool,
    /// A task notification arrived after the fire, before the next person
    /// prompt.
    pub wrong: bool,
    /// The task that made it wrong was never seen launched.
    pub unseen_launch: bool,
}

/// What the replay found.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub files: usize,
    pub stops: usize,
    pub continued_stops: usize,
    /// Stops whose tail had no context size (no usage yet).
    pub stops_without_size: usize,
    pub stops_past_size: usize,
    pub held_background: usize,
    pub held_relais: usize,
    /// Transcripts that could not be read.
    pub unreadable_files: usize,
    /// Stops whose tail could not be read back.
    pub unreadable_tails: usize,
    pub fires: Vec<Fire>,
    pub first_day: Option<String>,
    pub last_day: Option<String>,
}

const LAUNCHES: [&str; 4] = [
    "Command running in background with ID: ",
    "moved to the background (ID: ",
    "agentId: ",
    "Monitor started (task ",
];

/// The ids a tool result says were launched in the background.
fn launched_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for marker in LAUNCHES {
        let mut rest = text;
        while let Some(at) = rest.find(marker) {
            rest = &rest[at + marker.len()..];
            let id: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !id.is_empty() {
                ids.push(id);
            }
        }
    }
    ids
}

/// The task ids a text's `<task-notification>` blocks name.
fn notified_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("<task-id>") {
        rest = &rest[at + "<task-id>".len()..];
        if let Some(end) = rest.find("</task-id>") {
            ids.push(rest[..end].trim().to_string());
        }
    }
    ids
}

/// Every string in an entry that can carry a launch or a notification: tool
/// result texts, a string prompt, an attachment's prompt, a queue
/// operation's content.
fn texts_of(entry: &Value) -> Vec<String> {
    let mut texts = Vec::new();
    match entry.pointer("/message/content") {
        Some(Value::String(text)) => texts.push(text.clone()),
        Some(Value::Array(parts)) => {
            for part in parts {
                if part["type"].as_str() == Some("tool_result") {
                    match &part["content"] {
                        Value::String(text) => texts.push(text.clone()),
                        Value::Array(inner) => texts.extend(
                            inner
                                .iter()
                                .filter_map(|block| block["text"].as_str().map(str::to_string)),
                        ),
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }
    for pointer in ["/attachment/prompt", "/content"] {
        if let Some(text) = entry.pointer(pointer).and_then(Value::as_str) {
            texts.push(text.to_string());
        }
    }
    texts
}

fn assistant_text(entry: &Value) -> Option<String> {
    if entry_type(entry) != "assistant" || is_sidechain(entry) {
        return None;
    }
    let text: Vec<&str> = blocks(entry)
        .iter()
        .filter(|block| block["type"].as_str() == Some("text"))
        .filter_map(|block| block["text"].as_str())
        .collect();
    (!text.is_empty()).then(|| text.join("\n"))
}

fn is_stop(entry: &Value) -> bool {
    entry_type(entry) == "system"
        && entry["subtype"].as_str() == Some("stop_hook_summary")
        && !is_sidechain(entry)
}

fn day_of(entry: &Value) -> Option<String> {
    entry["timestamp"]
        .as_str()
        .filter(|stamp| stamp.len() >= 10)
        .map(|stamp| stamp[..10].to_string())
}

/// The entries of a file with the byte offset each line ends at.
fn lines_with_offsets(bytes: &[u8]) -> Vec<(u64, Value)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (at, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if let Ok(entry) = serde_json::from_slice::<Value>(&bytes[start..at]) {
                out.push((u64::try_from(at + 1).unwrap_or(u64::MAX), entry));
            }
            start = at + 1;
        }
    }
    out
}

impl Summary {
    /// Replay one transcript file into this self. `since` is the first day
    /// kept (`YYYY-MM-DD`); earlier Stops still carry the state forward.
    pub fn replay_file(&mut self, path: &Path, since: &str) {
        let Ok(bytes) = std::fs::read(path) else {
            self.unreadable_files += 1;
            return;
        };
        self.files += 1;
        let entries = lines_with_offsets(&bytes);
        let session = path
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        let mut state = State::default();
        let mut open: BTreeSet<String> = BTreeSet::new();
        // Every id seen launched, and every id already notified: one notification
        // is written several times (a queue operation, an attachment, a prompt),
        // and only its first copy says the task finished.
        let mut launched_ever: BTreeSet<String> = BTreeSet::new();
        let mut notified: BTreeSet<String> = BTreeSet::new();
        let mut mode: Option<String> = None;
        let mut last_text = String::new();
        let mut stops_in_turn = 0usize;
        let mut fired_any = false;
        // Fires awaiting the wrong check: the index into `self.fires` and the
        // ids launched after the fire, whose notifications do not count.
        let mut watching: Vec<(usize, BTreeSet<String>)> = Vec::new();
        for (end, entry) in &entries {
            // Any prompt opens a turn (a task notification too); only a person's
            // ends the wrong check.
            if is_prompt(entry) {
                stops_in_turn = 0;
            }
            if is_person_prompt(entry) {
                watching.clear();
            }
            if let Some(found) = entry["permissionMode"].as_str() {
                mode = Some(found.to_string());
            }
            for text in texts_of(entry) {
                for id in notified_ids(&text) {
                    open.remove(&id);
                    if !notified.insert(id.clone()) {
                        continue;
                    }
                    for (fire, after) in &watching {
                        if !after.contains(&id) {
                            self.fires[*fire].wrong = true;
                            if !launched_ever.contains(&id) {
                                self.fires[*fire].unseen_launch = true;
                            }
                        }
                    }
                }
                if entry_type(entry) == "user" {
                    let launched = launched_ids(&text);
                    for (_, after) in &mut watching {
                        after.extend(launched.iter().cloned());
                    }
                    launched_ever.extend(launched.iter().cloned());
                    open.extend(launched);
                }
            }
            if let Some(text) = assistant_text(entry) {
                last_text = text;
            }
            if !is_stop(entry) {
                continue;
            }
            let day = day_of(entry).unwrap_or_default();
            let counted = day.as_str() >= since;
            let handoff = handoff_line(&last_text);
            let view = StopView {
                cwd: entry["cwd"].as_str().map(PathBuf::from),
                home: None,
                permission_mode: mode.clone(),
                background_busy: !open.is_empty(),
                stop_hook_active: stops_in_turn > 0,
                ends_with_question: last_text.trim_end().ends_with('?'),
                waiting_line: last_text
                    .lines()
                    .any(|line| line.trim().starts_with("WAITING:")),
                marker: handoff.is_armed(),
                near_miss: crate::clear_boundary::near_miss_of(&handoff),
            };
            stops_in_turn += 1;
            let anchor = state.size_anchor.clone();
            let Ok(tail) = read_tail_until_at(path, *end, TAIL_FIRST_CHUNK, TAIL_CAP, |tail| {
                context_size(tail).is_some()
                    && anchor
                        .as_deref()
                        .is_none_or(|anchor| position_of(tail, anchor).is_some())
            }) else {
                self.unreadable_tails += usize::from(counted);
                continue;
            };
            if counted {
                self.stops += 1;
                self.continued_stops += usize::from(view.stop_hook_active);
                if self
                    .first_day
                    .as_deref()
                    .is_none_or(|first| day.as_str() < first)
                {
                    self.first_day = Some(day.clone());
                }
                if self
                    .last_day
                    .as_deref()
                    .is_none_or(|last| day.as_str() > last)
                {
                    self.last_day = Some(day.clone());
                }
            }
            let Some(size) = context_size(&tail.entries) else {
                self.stops_without_size += usize::from(counted);
                continue;
            };
            let mut saved = None;
            let verdict = decide(
                &view,
                &tail.entries,
                state.clone(),
                || PhaseFact::Working,
                (|| Stance::Deny, SIZE_BOUNDARY, || Stance::Deny),
                |_| Repositories::Clean { heads: Vec::new() },
                |next| {
                    saved = Some(next.clone());
                    true
                },
            );
            if let Some(next) = saved {
                state = next;
            }
            if !counted {
                continue;
            }
            if size >= SIZE_BOUNDARY {
                self.stops_past_size += 1;
            }
            match verdict.evidence.as_ref().map(|evidence| evidence.outcome) {
                Some(SizeOutcome::HeldBackground) => self.held_background += 1,
                Some(SizeOutcome::HeldRelais) => self.held_relais += 1,
                Some(
                    SizeOutcome::Requested
                    | SizeOutcome::Advised
                    | SizeOutcome::Observed
                    | SizeOutcome::Cap,
                )
                | None => {}
            }
            if matches!(verdict.decision, Decision::Continue(_)) {
                self.fires.push(Fire {
                    session: session.clone(),
                    day,
                    size,
                    continued: view.stop_hook_active,
                    first: !fired_any,
                    wrong: false,
                    unseen_launch: false,
                });
                fired_any = true;
                watching.push((self.fires.len() - 1, BTreeSet::new()));
            }
        }
    }
}

/// The main-chain transcripts under `root`: `<root>/<project>/<session>.jsonl`
/// (subagent transcripts live deeper and are left out).
pub fn transcripts(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(projects) = std::fs::read_dir(root) else {
        return files;
    };
    for project in projects.flatten() {
        let Ok(inner) = std::fs::read_dir(project.path()) else {
            continue;
        };
        for file in inner.flatten() {
            let path = file.path();
            if path.extension().is_some_and(|ext| ext == "jsonl") && path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Replay every transcript under `root` from `since` on.
pub fn replay(root: &Path, since: &str) -> Summary {
    let mut summary = Summary::default();
    for path in transcripts(root) {
        summary.replay_file(&path, since);
    }
    summary
}

impl Summary {
    /// Days from the first to the last Stop kept, at least 1.
    pub fn days(&self) -> u64 {
        let day = |text: &Option<String>| {
            text.as_deref()
                .and_then(|day| greenbelay_guard::civil::day_of(&format!("{day}T00:00:00Z")))
        };
        match (day(&self.first_day), day(&self.last_day)) {
            (Some(first), Some(last)) => u64::try_from(last - first).unwrap_or(0) + 1,
            _ => 1,
        }
    }

    /// The report: the bar first, then the counts behind it, coverage and
    /// fires per week, each line under 80 columns.
    pub fn render(&self) -> String {
        let days = self.days();
        let fires = self.fires.len();
        let wrong = self.fires.iter().filter(|fire| fire.wrong).count();
        let unseen = self.fires.iter().filter(|fire| fire.unseen_launch).count();
        let first: Vec<&Fire> = self.fires.iter().filter(|fire| fire.first).collect();
        let first_idle = first.iter().filter(|fire| !fire.wrong).count();
        let continued: Vec<&Fire> = self.fires.iter().filter(|fire| fire.continued).collect();
        let continued_wrong = continued.iter().filter(|fire| fire.wrong).count();
        let sessions: BTreeSet<&str> = self.fires.iter().map(|f| f.session.as_str()).collect();
        let mut weeks: BTreeMap<String, usize> = BTreeMap::new();
        for fire in &self.fires {
            let week = greenbelay_guard::civil::day_of(&format!("{}T00:00:00Z", fire.day))
                .map(|day| greenbelay_guard::civil::iso(greenbelay_guard::civil::week_start(day)))
                .unwrap_or_default();
            *weeks.entry(week).or_default() += 1;
        }
        let per_day = fires as f64 / days as f64;
        let wrong_share = if fires == 0 {
            0.0
        } else {
            wrong as f64 / fires as f64
        };
        let meets = per_day >= 1.0 && wrong_share <= 0.10;
        let lines = [
            format!(
                "bar: {} ({per_day:.2} fires a day, {:.1}% held wrongly;",
                if meets { "met" } else { "not met" },
                wrong_share * 100.0
            ),
            "  needs at least 1 a day and at most 10%)".to_string(),
            format!(
                "replayed: {} transcripts, {} Stops, {days} days ({} to {})",
                self.files,
                self.stops,
                self.first_day.as_deref().unwrap_or("-"),
                self.last_day.as_deref().unwrap_or("-"),
            ),
            format!(
                "unreadable: {} transcripts, {} Stop tails",
                self.unreadable_files, self.unreadable_tails
            ),
            format!("no context size: {} Stops", self.stops_without_size),
            format!(
                "not replayed: command boundaries past {}k (no phase fact)",
                SIZE_BOUNDARY / 1000
            ),
            "background: rebuilt from launch and notification texts".to_string(),
            format!(
                "past {}k: {} Stops; held: background {}, relais {}",
                SIZE_BOUNDARY / 1000,
                self.stops_past_size,
                self.held_background,
                self.held_relais
            ),
            format!("fires: {fires} in {} sessions", sessions.len()),
            format!(
                "  first in their session: {} ({first_idle} idle)",
                first.len()
            ),
            format!("  held wrongly: {wrong} ({unseen} launched where the replay cannot see)"),
            format!(
                "  on continued Stops: {}, held wrongly {continued_wrong}",
                continued.len()
            ),
        ];
        let mut out = String::new();
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        for (week, count) in &weeks {
            out.push_str(&format!("  week of {week}: {count}\n"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn prompt(uuid: &str, day: &str) -> Value {
        json!({"type": "user", "uuid": uuid, "timestamp": format!("{day}T10:00:00Z"),
            "permissionMode": "bypassPermissions", "message": {"role": "user", "content": "go"}})
    }

    fn assistant(uuid: &str, size: u64, text: &str) -> Value {
        json!({"type": "assistant", "uuid": uuid, "isSidechain": false, "message": {"role": "assistant",
            "usage": {"input_tokens": size, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0},
            "content": [{"type": "text", "text": text}]}})
    }

    fn stop(uuid: &str, day: &str) -> Value {
        json!({"type": "system", "subtype": "stop_hook_summary", "uuid": uuid, "isSidechain": false,
            "timestamp": format!("{day}T10:05:00Z"), "cwd": "/work"})
    }

    fn launched(id: &str) -> Value {
        json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result",
            "tool_use_id": "t", "content": [{"type": "text",
            "text": format!("Command running in background with ID: {id}. Output is being written")}]}]}})
    }

    fn notification(id: &str) -> Value {
        json!({"type": "attachment", "attachment": {"type": "queued_command",
            "prompt": format!("<task-notification>\n<task-id>{id}</task-id>\n</task-notification>")}})
    }

    fn write(name: &str, entries: &[Value]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("size-replay-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.jsonl"));
        let text: String = entries.iter().map(|e| format!("{e}\n")).collect();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn run(name: &str, entries: &[Value]) -> Summary {
        let path = write(name, entries);
        let mut summary = Summary::default();
        summary.replay_file(&path, "2026-01-01");
        let _ = std::fs::remove_dir_all(path.parent().unwrap()); // a scratch directory; a leftover is harmless
        summary
    }

    #[test]
    fn an_idle_stop_past_the_size_fires_once_and_a_continued_stop_is_judged() {
        let summary = run(
            "idle",
            &[
                prompt("p1", "2026-11-20"),
                assistant("a1", 450_000, "working"),
                stop("s1", "2026-11-20"),
                assistant("a2", 455_000, "still working"),
                stop("s2", "2026-11-20"),
            ],
        );
        assert_eq!(summary.stops, 2);
        assert_eq!(summary.continued_stops, 1);
        assert_eq!(summary.fires.len(), 1);
        assert!(summary.fires[0].first && !summary.fires[0].wrong);
    }

    #[test]
    fn a_background_task_holds_until_its_notification() {
        let summary = run(
            "busy",
            &[
                prompt("p1", "2026-11-20"),
                launched("b1"),
                assistant("a1", 450_000, "waiting on the build"),
                stop("s1", "2026-11-20"),
                notification("b1"),
                assistant("a2", 452_000, "built"),
                stop("s2", "2026-11-20"),
            ],
        );
        assert_eq!(summary.held_background, 1);
        assert_eq!(summary.fires.len(), 1);
        assert!(!summary.fires[0].wrong);
    }

    #[test]
    fn a_notification_after_a_fire_marks_it_held_wrongly() {
        let summary = run(
            "missed",
            &[
                prompt("p1", "2026-11-20"),
                assistant("a1", 450_000, "done"),
                stop("s1", "2026-11-20"),
                notification("unknown"),
            ],
        );
        assert_eq!(summary.fires.len(), 1);
        assert!(summary.fires[0].wrong);
    }

    #[test]
    fn a_task_launched_after_the_fire_is_not_held_wrongly() {
        let summary = run(
            "after",
            &[
                prompt("p1", "2026-11-20"),
                assistant("a1", 450_000, "done"),
                stop("s1", "2026-11-20"),
                launched("later"),
                notification("later"),
            ],
        );
        assert_eq!(summary.fires.len(), 1);
        assert!(!summary.fires[0].wrong);
    }

    #[test]
    fn a_notification_opens_a_turn_so_its_stop_is_not_continued() {
        let note = json!({"type": "user", "uuid": "n1", "message": {"role": "user",
            "content": "<task-notification>\n<task-id>z</task-id>\n</task-notification>"}});
        let summary = run(
            "turns",
            &[
                prompt("p1", "2026-11-20"),
                assistant("a1", 100_000, "x"),
                stop("s1", "2026-11-20"),
                note,
                assistant("a2", 100_000, "y"),
                stop("s2", "2026-11-20"),
            ],
        );
        assert_eq!((summary.stops, summary.continued_stops), (2, 0));
    }

    #[test]
    fn a_question_and_plan_mode_are_silent() {
        let mut plan = prompt("p1", "2026-11-20");
        plan["permissionMode"] = json!("plan");
        let summary = run(
            "silent",
            &[
                plan,
                assistant("a1", 450_000, "plan ready"),
                stop("s1", "2026-11-20"),
                prompt("p2", "2026-11-20"),
                assistant("a2", 460_000, "Shall I go on?"),
                stop("s2", "2026-11-20"),
            ],
        );
        assert!(summary.fires.is_empty());
    }

    #[test]
    fn launch_and_notification_ids_are_read() {
        assert_eq!(
            launched_ids("Async agent launched successfully.\nagentId: ade39ae26a34 (internal"),
            vec!["ade39ae26a34".to_string()]
        );
        assert_eq!(
            launched_ids("was moved to the background (ID: barjqnez1). Output"),
            vec!["barjqnez1".to_string()]
        );
        assert_eq!(
            notified_ids("<task-notification>\n<task-id>x1</task-id>"),
            vec!["x1".to_string()]
        );
    }
}
