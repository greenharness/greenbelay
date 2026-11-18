//! `greenbelay explain clear-at-boundary --format cases`: freezes the Stops
//! the rule's journal lines record as fixtures a corpus can replay
//! (`docs/plans/2026-11-18-reviewed-stop-cases.md`, Behaviour section 3).
//!
//! A journal line says that a Stop reached a boundary and what the rule did
//! with it. This module finds the Stop in the session's transcript, rebuilds
//! the byte window the hook read there (the same first chunk, cap and
//! stopping predicate as [`on_stop`](crate::clear_boundary::on_stop)), freezes
//! the facts the outcome implies, trims the entries to what the judgement
//! reads and replays the result through the hook's own core. The fixture is
//! countable only when the original window and the trimmed one agree.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use greenbelay_guard::atomic::write_atomic;
use greenbelay_guard::civil;
use greenbelay_guard::corpus;
use greenbelay_guard::journal;
use greenbelay_guard::rules::clear_at_boundary::RULE;
use serde_json::{json, Map, Value};

use crate::clear_boundary::{
    blocks, boundaries, entry_type, is_person_prompt, is_sidechain, judge_case, pending_from,
    read_tail_until_at, relais_messages, replay_at_deny, turn_window, window_is_enough, word_after,
    work_in, Pending, Repositories, State, StopView, Tail, TAIL_CAP, TAIL_FIRST_CHUNK,
};
use crate::plan_phases::PhaseFact;

/// What one export reads and writes.
pub struct Options {
    /// The journal files, oldest first.
    pub journal: Vec<PathBuf>,
    /// Every transcript file under the roots.
    pub transcripts: Vec<PathBuf>,
    /// Where the fixtures are written.
    pub out: PathBuf,
    /// The target `.cases` file's text, `""` when there is none.
    pub cases_text: String,
    /// Keep only the most recent N Stops.
    pub sample: Option<usize>,
    /// The home directory, which becomes `~`.
    pub home: Option<String>,
    /// The hook's first tail chunk and its cap; the defaults are the hook's.
    pub first_chunk: u64,
    pub cap: u64,
}

impl Options {
    pub fn new(out: PathBuf) -> Self {
        Self {
            journal: Vec::new(),
            transcripts: Vec::new(),
            out,
            cases_text: String::new(),
            sample: None,
            home: None,
            first_chunk: TAIL_FIRST_CHUNK,
            cap: TAIL_CAP,
        }
    }
}

/// What an export did.
#[derive(Debug, Default)]
pub struct Report {
    pub dir: PathBuf,
    /// Candidate Stops found in the journal.
    pub candidates: usize,
    pub written: usize,
    pub replay_differs: usize,
    pub excluded: usize,
    pub already: usize,
    pub skipped: usize,
    /// One line each for a Stop skipped, for stderr.
    pub notes: Vec<String>,
    /// The case lines, for stdout.
    pub stdout: Vec<String>,
}

impl Report {
    pub fn summary(&self) -> String {
        format!(
            "wrote {} fixtures to {} ({} replay differs, {} excluded, {} already exported, {} skipped)",
            self.written,
            self.dir.display(),
            self.replay_differs,
            self.excluded,
            self.already,
            self.skipped
        )
    }

    /// Candidates were found and every one was skipped.
    pub fn failed(&self) -> bool {
        self.candidates > 0 && self.skipped == self.candidates
    }
}

/// What a journal line says happened at a Stop.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Advised,
    Denied,
    PendingWork,
    PendingRelais,
    PendingBackground,
    NotClean(String),
}

impl Kind {
    fn of(outcome: &str, body: &str) -> Option<Kind> {
        match (outcome, body) {
            ("advised", _) => Some(Kind::Advised),
            ("denied", _) => Some(Kind::Denied),
            ("watched", "pending-work") => Some(Kind::PendingWork),
            ("watched", "pending background") => Some(Kind::PendingBackground),
            ("watched", _) if body.starts_with("pending relais") => Some(Kind::PendingRelais),
            ("watched", _) => body
                .strip_prefix("pending ")
                .map(|reason| Kind::NotClean(reason.to_string())),
            _ => None,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Kind::Advised => "advised",
            Kind::Denied => "denied",
            Kind::PendingWork => "pending-work",
            Kind::PendingRelais => "pending relais",
            Kind::PendingBackground => "pending background",
            Kind::NotClean(_) => "pending not-clean",
        }
    }

    fn expects_fire(&self) -> bool {
        matches!(self, Kind::Advised | Kind::Denied)
    }

    /// A hold saves the boundary as pending for a later Stop.
    fn is_hold(&self) -> bool {
        matches!(
            self,
            Kind::PendingRelais | Kind::PendingBackground | Kind::NotClean(_)
        )
    }

    /// Whether the replay's journal reason is the one the line recorded.
    fn matches_reason(&self, reason: &str) -> bool {
        match self {
            Kind::Advised | Kind::Denied => true,
            Kind::PendingWork => reason == "pending-work",
            Kind::PendingBackground => reason == "pending background",
            Kind::PendingRelais => reason.starts_with("pending relais"),
            Kind::NotClean(why) => reason == format!("pending {why}"),
        }
    }
}

/// One journal line of the rule.
struct Row {
    ts: u64,
    session: String,
    mode: String,
    body: String,
    uuid: Option<String>,
    capped: bool,
    kind: Option<Kind>,
}

/// The fixed fields of a live `F` line of this rule, with the excerpt's
/// ` tail-capped` and ` uuid=<id>` suffixes taken off.
fn parse_row(line: &str) -> Option<Row> {
    let mut fields = line.strip_prefix("F ")?.splitn(9, ' ');
    let ts = fields.next()?.parse::<u64>().ok()?;
    let rule = fields.next()?;
    let _stance = fields.next()?;
    let outcome = fields.next()?;
    let session = fields.next()?;
    let _repo = fields.next()?;
    let mode = fields.next()?;
    let arm = fields.next()?;
    let excerpt = fields.next().unwrap_or("").trim();
    if rule != RULE.id || arm == "arm=shadow" {
        return None;
    }
    let (rest, uuid) = match excerpt.rsplit_once(' ') {
        Some((rest, last)) if last.starts_with("uuid=") => {
            (rest, last.strip_prefix("uuid=").map(str::to_string))
        }
        _ if excerpt.starts_with("uuid=") => {
            ("", excerpt.strip_prefix("uuid=").map(str::to_string))
        }
        _ => (excerpt, None),
    };
    let (body, capped) = match rest.strip_suffix(" tail-capped") {
        Some(body) => (body, true),
        None if rest == "tail-capped" => ("", true),
        None => (rest, false),
    };
    Some(Row {
        ts,
        session: session.to_string(),
        mode: mode.to_string(),
        body: body.to_string(),
        uuid,
        capped,
        kind: Kind::of(outcome, body),
    })
}

/// The journal files in `dir`: `journal.log`, its generations and any copy
/// kept beside them, in name order.
pub fn journal_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("journal.log")
                })
                .map(|entry| entry.path())
                .filter(|path| path.is_file())
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// The rows of `paths`; a line a later file repeats from an earlier one (a
/// rotated copy) counts once.
fn read_rows(paths: &[PathBuf]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut earlier: HashSet<String> = HashSet::new();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        rows.extend(
            lines
                .iter()
                .filter(|line| !earlier.contains(**line))
                .filter_map(|line| parse_row(line)),
        );
        earlier.extend(lines.iter().map(|line| (*line).to_string()));
    }
    rows.sort_by_key(|row| row.ts); // stable: lines of one second keep their file order
    rows
}

/// A transcript line: where it ends, and what finds a Stop in it.
struct LineIndex {
    end: u64,
    uuid: Option<String>,
    seconds: Option<u64>,
    main_chain: bool,
}

/// The seconds since the epoch of an RFC 3339 timestamp in UTC.
fn epoch_seconds(timestamp: &str) -> Option<u64> {
    let day = civil::day_of(timestamp)?;
    let digits = |from: usize, to: usize| -> Option<u64> { timestamp.get(from..to)?.parse().ok() };
    let (hours, minutes, seconds) = (digits(11, 13)?, digits(14, 16)?, digits(17, 19)?);
    u64::try_from(day)
        .ok()
        .map(|days| days * 86_400 + hours * 3_600 + minutes * 60 + seconds)
}

fn index_transcript(path: &Path) -> std::io::Result<Vec<LineIndex>> {
    let mut reader = BufReader::new(std::fs::File::open(path)?);
    let mut lines = Vec::new();
    let mut offset = 0u64;
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 {
            return Ok(lines);
        }
        offset += read as u64;
        // A line that is not JSON is skipped by the tail reader too; it is a
        // line no Stop can sit on.
        let entry: Value = serde_json::from_slice(&buffer).unwrap_or(Value::Null);
        lines.push(LineIndex {
            end: offset,
            uuid: entry["uuid"].as_str().map(str::to_string),
            seconds: entry["timestamp"].as_str().and_then(epoch_seconds),
            main_chain: !is_sidechain(&entry),
        });
    }
}

/// Why a Stop could not be placed in a transcript.
enum Unplaced {
    /// No transcript, or more than one: the Stop is skipped.
    Skipped(String),
    /// The transcript is there but the window cannot be rebuilt: the fixture
    /// is written, not countable.
    Unbuildable(String),
}

/// A Stop placed in its transcript with the hook's window rebuilt.
struct Placed {
    end: u64,
    path: PathBuf,
    tail: Tail,
    stop_uuid: Option<String>,
    cwd: Option<String>,
}

/// The facts and state frozen for one Stop.
struct Frozen {
    view: StopView,
    prior: State,
    phase: PhaseFact,
    repositories: Repositories,
}

/// What building one Stop's fixture came to.
enum Built {
    Skipped(String),
    Fixture(Box<Fixture>),
}

struct Fixture {
    name: String,
    text: String,
    /// Why the fixture is not countable.
    excluded: Option<String>,
    /// How the replay at deny disagrees with the journal line.
    replay_differs: Option<String>,
}

/// Path scrubbing and command redaction.
struct Scrub {
    home: Option<String>,
}

impl Scrub {
    /// `$HOME` and any other `/Users/<name>` become `~`.
    fn text(&self, text: &str) -> String {
        let mut scrubbed = text.to_string();
        if let Some(home) = self.home.as_deref().filter(|home| home.len() > 1) {
            scrubbed = scrubbed.replace(home, "~");
        }
        const USERS: &str = "/Users/";
        while let Some(at) = scrubbed.find(USERS) {
            let name_start = at + USERS.len();
            let name_len = scrubbed[name_start..]
                .find(|c: char| {
                    c == '/' || c.is_whitespace() || matches!(c, '"' | '\'' | ':' | ')')
                })
                .unwrap_or(scrubbed.len() - name_start);
            scrubbed.replace_range(at..name_start + name_len, "~");
        }
        scrubbed
    }

    /// A Bash command, through the backtest's redaction.
    fn command(&self, command: &str) -> String {
        self.text(&journal::redact(command))
    }
}

/// Which window was placed: the row index and the pending uuid it grew to.
type PlacedKey = (usize, Option<String>);

/// The exporter: the journal's rows, the transcripts and what was read of
/// them.
struct Exporter<'a> {
    options: &'a Options,
    rows: Vec<Row>,
    scrub: Scrub,
    indexes: HashMap<PathBuf, Vec<LineIndex>>,
    after_hold: HashMap<usize, Option<Pending>>,
    /// The last window placed: a hold is placed to find what it left
    /// pending and again to export it.
    last_placed: Option<(PlacedKey, Rc<Placed>)>,
}

impl Exporter<'_> {
    fn transcript_for(&self, session: &str) -> Result<PathBuf, Unplaced> {
        let matching: Vec<&PathBuf> = self
            .options
            .transcripts
            .iter()
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "jsonl")
                    && path
                        .file_stem()
                        .is_some_and(|stem| stem.to_string_lossy().starts_with(session))
            })
            .collect();
        match matching.as_slice() {
            [] => Err(Unplaced::Skipped(format!(
                "session {session}: no transcript"
            ))),
            [only] => Ok((*only).clone()),
            _ => Err(Unplaced::Skipped(format!(
                "session {session}: the prefix matches {} transcripts",
                matching.len()
            ))),
        }
    }

    /// Find row `at`'s Stop in its transcript and read the window the hook
    /// read there, with `pending_uuid` as the hook's prior pending boundary.
    fn place(&mut self, at: usize, pending_uuid: Option<&str>) -> Result<Rc<Placed>, Unplaced> {
        let key = (at, pending_uuid.map(str::to_string));
        if let Some((cached, placed)) = &self.last_placed {
            if *cached == key {
                return Ok(Rc::clone(placed));
            }
        }
        let placed = Rc::new(self.read_window(at, pending_uuid)?);
        self.last_placed = Some((key, Rc::clone(&placed)));
        Ok(placed)
    }

    fn read_window(&mut self, at: usize, pending_uuid: Option<&str>) -> Result<Placed, Unplaced> {
        let (session, uuid, ts) = {
            let row = &self.rows[at];
            (row.session.clone(), row.uuid.clone(), row.ts)
        };
        if session.is_empty() || session == "-" {
            return Err(Unplaced::Skipped("a line without a session".to_string()));
        }
        let path = self.transcript_for(&session)?;
        if !self.indexes.contains_key(&path) {
            let index = index_transcript(&path).map_err(|error| {
                Unplaced::Skipped(format!("session {session}: transcript unreadable: {error}"))
            })?;
            self.indexes.insert(path.clone(), index);
        }
        let lines = &self.indexes[&path];
        let line = match &uuid {
            Some(id) => lines
                .iter()
                .position(|line| line.uuid.as_deref() == Some(id)),
            None => lines
                .iter()
                .rposition(|line| line.main_chain && line.seconds.is_some_and(|s| s <= ts)),
        }
        .ok_or_else(|| {
            Unplaced::Unbuildable(match &uuid {
                Some(id) => format!("no entry uuid={id} in the transcript"),
                None => "no main-chain entry at or before the journal second".to_string(),
            })
        })?;
        let end = lines[line].end;
        let stop_uuid = lines[line].uuid.clone();
        let tail = read_tail_until_at(
            &path,
            end,
            self.options.first_chunk,
            self.options.cap,
            |entries| window_is_enough(entries, pending_uuid),
        )
        .map_err(|error| Unplaced::Unbuildable(format!("window unreadable: {error}")))?;
        if tail.entries.is_empty() {
            return Err(Unplaced::Unbuildable(
                "the window holds no entry".to_string(),
            ));
        }
        let cwd = tail
            .entries
            .iter()
            .rev()
            .find_map(|entry| entry["cwd"].as_str())
            .map(str::to_string);
        Ok(Placed {
            end,
            path,
            tail,
            stop_uuid,
            cwd,
        })
    }

    /// The boundary the session had pending when row `target` was judged:
    /// what the latest earlier hold left pending, unless an advice or a "no
    /// boundary" cleared it since.
    fn standing_pending(&mut self, target: usize) -> Option<Pending> {
        let session = self.rows[target].session.clone();
        for at in (0..target).rev() {
            let row = &self.rows[at];
            if row.session != session {
                continue;
            }
            match &row.kind {
                Some(kind) if kind.is_hold() => return self.pending_after(at),
                Some(Kind::Advised | Kind::Denied) => return None,
                _ if row.body.starts_with("no boundary")
                    || row.body.starts_with("pending-lost") =>
                {
                    return None
                }
                _ => {}
            }
        }
        None
    }

    /// The boundary pending after hold row `hold`: the one its own turn
    /// reached, or the one it carried over from the hold before it.
    // holds-until: a session's holds fit in memory and a few hundred window
    // reads; past that, the pending boundaries are written beside the fixtures.
    fn pending_after(&mut self, hold: usize) -> Option<Pending> {
        if let Some(known) = self.after_hold.get(&hold) {
            return known.clone();
        }
        let before = self.standing_pending(hold);
        let pending = match self.place(hold, before.as_ref().map(|p| p.uuid.as_str())) {
            Err(_) => None,
            Ok(placed) => {
                let cwd = PathBuf::from(placed.cwd.clone().unwrap_or_default());
                let own = turn_window(&placed.tail.entries).and_then(|window| {
                    let found = boundaries(&window, &cwd);
                    let boundary = found.last()?;
                    before
                        .as_ref()
                        .is_none_or(|before| before.uuid != boundary.uuid)
                        .then(|| pending_from(&found, boundary))
                });
                own.or(before)
            }
        };
        self.after_hold.insert(hold, pending.clone());
        pending
    }

    fn advices_before(&self, target: usize) -> u32 {
        let session = &self.rows[target].session;
        let count = self.rows[..target]
            .iter()
            .filter(|row| {
                row.session == *session && matches!(row.kind, Some(Kind::Advised | Kind::Denied))
            })
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    fn build(&mut self, target: usize) -> Built {
        let Some(kind) = self.rows[target].kind.clone() else {
            return Built::Skipped("not a Stop".to_string());
        };
        let name = format!(
            "{}-{}",
            self.rows[target]
                .session
                .chars()
                .take(8)
                .collect::<String>(),
            self.rows[target].ts
        );
        let pending = self.standing_pending(target);
        let pending_uuid = pending.as_ref().map(|pending| pending.uuid.clone());
        let placed = match self.place(target, pending_uuid.as_deref()) {
            Ok(placed) => placed,
            Err(Unplaced::Skipped(why)) => return Built::Skipped(why),
            Err(Unplaced::Unbuildable(why)) => {
                return Built::Fixture(Box::new(self.unbuildable(target, &kind, &name, &why)))
            }
        };
        if kind == Kind::PendingWork && pending.is_none() {
            return Built::Fixture(Box::new(self.unbuildable(
                target,
                &kind,
                &name,
                "no earlier hold to rebuild the pending boundary from",
            )));
        }
        let row = &self.rows[target];
        if row.capped != placed.tail.capped {
            let why = format!(
                "the rebuilt window is {}capped, the journal line says {}",
                if placed.tail.capped { "" } else { "not " },
                if row.capped {
                    "tail-capped"
                } else {
                    "not capped"
                }
            );
            return Built::Fixture(Box::new(self.unbuildable(target, &kind, &name, &why)));
        }
        let frozen = Frozen {
            view: StopView {
                cwd: placed.cwd.as_ref().map(PathBuf::from),
                permission_mode: Some(row.mode.clone()).filter(|mode| mode != "-"),
                background_busy: kind == Kind::PendingBackground,
                stop_hook_active: false,
                ends_with_question: false,
                waiting_line: false,
                marker: false,
            },
            prior: State {
                turn: String::new(),
                key: String::new(),
                advices: self.advices_before(target),
                pending,
            },
            phase: PhaseFact::NoOpenPhase,
            repositories: match &kind {
                Kind::NotClean(reason) => Repositories::NotClean {
                    directory: PathBuf::new(),
                    reason: reason.clone(),
                },
                _ => Repositories::Clean { heads: Vec::new() },
            },
        };
        let early = early_after(&placed.path, placed.end).unwrap_or(false);
        Built::Fixture(Box::new(
            self.freeze(target, &kind, name, &placed, &frozen, early),
        ))
    }

    /// A fixture for a Stop whose window cannot be rebuilt: no entries, not
    /// countable.
    fn unbuildable(&self, target: usize, kind: &Kind, name: &str, why: &str) -> Fixture {
        let head = self.head(
            target,
            kind,
            &Frozen {
                view: StopView::default(),
                prior: State::default(),
                phase: PhaseFact::NoOpenPhase,
                repositories: Repositories::Clean { heads: Vec::new() },
            },
            None,
            false,
            Some(why),
        );
        Fixture {
            name: name.to_string(),
            text: render(&head, &[]),
            excluded: Some(why.to_string()),
            replay_differs: None,
        }
    }

    /// The fixture's members before its entries.
    fn head(
        &self,
        target: usize,
        kind: &Kind,
        frozen: &Frozen,
        placed: Option<&Placed>,
        early: bool,
        excluded: Option<&str>,
    ) -> Map<String, Value> {
        let row = &self.rows[target];
        let text = |value: &str| json!(self.scrub.text(value));
        let pending = frozen
            .prior
            .pending
            .as_ref()
            .map_or(Value::Null, |pending| {
                json!({
                    "uuid": pending.uuid,
                    "command": self.scrub.command(&pending.command),
                    "what": pending.what,
                    "dirs": pending
                        .dirs
                        .iter()
                        .map(|dir| self.scrub.text(&dir.to_string_lossy()))
                        .collect::<Vec<_>>(),
                })
            });
        let mut head = Map::new();
        head.insert(
            "source".to_string(),
            json!({
                "session": row.session.chars().take(8).collect::<String>(),
                "journal": row.ts,
                "outcome": kind.label(),
                "what": self.scrub.text(&row.body),
                "stop_uuid": placed.and_then(|placed| placed.stop_uuid.clone()),
                "window": placed.map(|placed| json!({
                    "start": placed.tail.start,
                    "end": placed.tail.end,
                    "capped": placed.tail.capped,
                })),
            }),
        );
        head.insert(
            "payload".to_string(),
            json!({
                "cwd": frozen.view.cwd.as_ref().map(|cwd| text(&cwd.to_string_lossy())),
                "permission_mode": frozen.view.permission_mode,
                "background_busy": frozen.view.background_busy,
                "stop_hook_active": frozen.view.stop_hook_active,
                "ends_with_question": frozen.view.ends_with_question,
                "waiting_line": frozen.view.waiting_line,
                "marker": frozen.view.marker,
            }),
        );
        if *kind == Kind::PendingBackground {
            head.insert("inferred".to_string(), json!(true));
        }
        head.insert(
            "state".to_string(),
            json!({
                "turn": frozen.prior.turn,
                "key": frozen.prior.key,
                "advices": frozen.prior.advices,
                "pending": pending,
            }),
        );
        head.insert(
            "facts".to_string(),
            json!({
                "phase": "none",
                "repositories": match &frozen.repositories {
                    Repositories::NotClean { reason, .. } => json!({ "not_clean": reason }),
                    Repositories::TimedOut => json!("timed_out"),
                    Repositories::Clean { .. } => json!("clean"),
                },
            }),
        );
        head.insert("early".to_string(), json!(early));
        head.insert("countable".to_string(), json!(excluded.is_none()));
        if let Some(why) = excluded {
            head.insert("excluded".to_string(), json!(why));
        }
        head
    }

    /// Trim, replay both windows, and write the fixture's text.
    fn freeze(
        &self,
        target: usize,
        kind: &Kind,
        name: String,
        placed: &Placed,
        frozen: &Frozen,
        early: bool,
    ) -> Fixture {
        let original = replay_at_deny(
            &frozen.view,
            &placed.tail.entries,
            frozen.prior.clone(),
            frozen.phase,
            frozen.repositories.clone(),
        );
        let entries: Vec<Value> = placed
            .tail
            .entries
            .iter()
            .filter_map(|entry| self.trim(entry))
            .collect();
        let scrubbed = Frozen {
            view: StopView {
                cwd: frozen
                    .view
                    .cwd
                    .as_ref()
                    .map(|cwd| PathBuf::from(self.scrub.text(&cwd.to_string_lossy()))),
                ..frozen.view.clone()
            },
            prior: State {
                pending: frozen.prior.pending.as_ref().map(|pending| Pending {
                    command: self.scrub.command(&pending.command),
                    dirs: pending
                        .dirs
                        .iter()
                        .map(|dir| PathBuf::from(self.scrub.text(&dir.to_string_lossy())))
                        .collect(),
                    ..pending.clone()
                }),
                ..frozen.prior.clone()
            },
            phase: frozen.phase,
            repositories: frozen.repositories.clone(),
        };
        let redacted = replay_at_deny(
            &scrubbed.view,
            &entries,
            scrubbed.prior.clone(),
            scrubbed.phase,
            scrubbed.repositories.clone(),
        );
        let disagreement = self.disagreement(&original, &redacted);
        let write = |excluded: Option<&str>| {
            render(
                &self.head(target, kind, &scrubbed, Some(placed), early, excluded),
                &entries,
            )
        };
        let mut excluded = disagreement;
        let mut text = write(excluded.as_deref());
        if excluded.is_none() {
            excluded = match judge_case(&text) {
                Ok(fired) if fired == redacted.fired => None,
                Ok(_) => {
                    Some("the fixture replays differently from its in-memory replay".to_string())
                }
                Err(why) => Some(format!("the fixture does not replay: {why}")),
            };
            if excluded.is_some() {
                text = write(excluded.as_deref());
            }
        }
        let replay_differs = if excluded.is_some() {
            None
        } else {
            replay_difference(kind, &redacted)
        };
        Fixture {
            name,
            text,
            excluded,
            replay_differs,
        }
    }

    /// Whether the original-window and redacted-window replays agree: the
    /// decision, the journal reasons and the state written, after the paths
    /// and commands are normalised.
    fn disagreement(
        &self,
        original: &crate::clear_boundary::Replay,
        redacted: &crate::clear_boundary::Replay,
    ) -> Option<String> {
        if original.fired != redacted.fired {
            return Some(format!(
                "the redacted window {} where the original {}",
                fires(redacted.fired),
                fires(original.fired)
            ));
        }
        if original.records != redacted.records {
            return Some("the redacted window journals other reasons".to_string());
        }
        // The original window's commands are normalised the way the trimmed
        // entries were; the redacted window's already are.
        let normal = |state: &Option<State>, original_window: bool| {
            let command = |text: &str| {
                if original_window {
                    self.scrub.command(text)
                } else {
                    text.to_string()
                }
            };
            state.as_ref().map(|state| {
                (
                    state.turn.clone(),
                    command(&state.key),
                    state.advices,
                    state.pending.as_ref().map(|pending| {
                        (
                            pending.uuid.clone(),
                            command(&pending.command),
                            pending.what.clone(),
                        )
                    }),
                )
            })
        };
        (normal(&original.saved, true) != normal(&redacted.saved, false))
            .then(|| "the redacted window writes another state".to_string())
    }

    /// An entry cut to what the judgement reads; `None` for an entry it never
    /// reads.
    fn trim(&self, entry: &Value) -> Option<Value> {
        let kind = entry_type(entry);
        if kind != "user" && kind != "assistant" {
            return None;
        }
        let mut trimmed = Map::new();
        trimmed.insert("type".to_string(), json!(kind));
        if let Some(uuid) = entry["uuid"].as_str() {
            trimmed.insert("uuid".to_string(), json!(uuid));
        }
        for flag in ["isSidechain", "isMeta"] {
            if let Some(value) = entry[flag].as_bool() {
                trimmed.insert(flag.to_string(), json!(value));
            }
        }
        let mut message = Map::new();
        if kind == "assistant" {
            if let Some(usage) = entry.pointer("/message/usage").and_then(Value::as_object) {
                let kept: Map<String, Value> = [
                    "input_tokens",
                    "cache_creation_input_tokens",
                    "cache_read_input_tokens",
                ]
                .into_iter()
                .filter_map(|field| {
                    usage
                        .get(field)
                        .filter(|value| value.is_u64())
                        .map(|value| (field.to_string(), value.clone()))
                })
                .collect();
                message.insert("usage".to_string(), Value::Object(kept));
            }
            if entry
                .pointer("/message/content")
                .is_some_and(Value::is_array)
            {
                let kept: Vec<Value> = blocks(entry)
                    .iter()
                    .map(|block| self.trim_block(block))
                    .collect();
                message.insert("content".to_string(), Value::Array(kept));
            }
        } else if let Some(content) = self.trim_user_content(entry) {
            message.insert("content".to_string(), content);
        }
        trimmed.insert("message".to_string(), Value::Object(message));
        Some(Value::Object(trimmed))
    }

    fn trim_block(&self, block: &Value) -> Value {
        let kind = block["type"].as_str().unwrap_or("");
        if kind != "tool_use" {
            return json!({ "type": kind });
        }
        let name = block["name"].as_str().unwrap_or("");
        let mut kept = Map::new();
        kept.insert("type".to_string(), json!("tool_use"));
        if let Some(id) = block["id"].as_str() {
            kept.insert("id".to_string(), json!(id));
        }
        kept.insert("name".to_string(), json!(name));
        match name {
            "Bash" => {
                if let Some(command) = block.pointer("/input/command").and_then(Value::as_str) {
                    kept.insert(
                        "input".to_string(),
                        json!({ "command": self.scrub.command(command) }),
                    );
                }
            }
            "mcp__relais__decide" => {
                if let Some(run) = block.pointer("/input/run").and_then(Value::as_str) {
                    kept.insert("input".to_string(), json!({ "run": run }));
                }
            }
            _ => {}
        }
        Value::Object(kept)
    }

    /// A user entry's content: a stub that keeps what makes the entry a
    /// prompt, an injected prompt, a plugin message or a tool result.
    fn trim_user_content(&self, entry: &Value) -> Option<Value> {
        match entry.pointer("/message/content")? {
            Value::String(text) => Some(json!(trim_text(text))),
            Value::Array(parts) => {
                let results: Vec<Value> = parts
                    .iter()
                    .filter(|part| part["type"].as_str() == Some("tool_result"))
                    .map(|part| {
                        let mut kept = Map::new();
                        kept.insert("type".to_string(), json!("tool_result"));
                        if let Some(id) = part["tool_use_id"].as_str() {
                            kept.insert("tool_use_id".to_string(), json!(id));
                        }
                        kept.insert("content".to_string(), json!(result_stub(part)));
                        Value::Object(kept)
                    })
                    .collect();
                if !results.is_empty() {
                    return Some(Value::Array(results));
                }
                let first_text = parts.iter().find_map(|part| part["text"].as_str());
                Some(json!(first_text.map_or("<text>".to_string(), trim_text)))
            }
            _ => None,
        }
    }
}

fn fires(fired: bool) -> &'static str {
    if fired {
        "fires"
    } else {
        "is silent"
    }
}

/// How the replay at deny differs from what the journal line recorded.
fn replay_difference(kind: &Kind, replay: &crate::clear_boundary::Replay) -> Option<String> {
    if replay.fired != kind.expects_fire() {
        return Some(format!(
            "journal says {}, the replay at deny {}",
            kind.label(),
            if replay.fired {
                "fires"
            } else {
                "does not fire"
            }
        ));
    }
    if !kind.expects_fire() {
        let reason = replay.records.last().map_or("", |(_, what)| what.as_str());
        if !kind.matches_reason(reason) {
            return Some(format!(
                "journal says {}, the replay says `{reason}`",
                kind.label()
            ));
        }
    }
    None
}

/// What a string user content becomes: a relais plugin message keeps the
/// parts [`RelaisFlight`](crate::clear_boundary::RelaisFlight) reads, an
/// injected prompt its marker, anything else a placeholder.
fn trim_text(text: &str) -> String {
    use relais_messages::{
        EXITED, FINISHED, PLUGIN_MESSAGE_PREFIX, RUN_ID, SUCCESSOR, WITHOUT_OUTCOME,
    };
    if let Some(message) = text.strip_prefix(PLUGIN_MESSAGE_PREFIX) {
        let mut kept = String::new();
        if message.contains(SUCCESSOR) {
            kept.push_str(SUCCESSOR);
            kept.push('\n');
        }
        if let Some(id) = word_after(message, RUN_ID).filter(|id| id.starts_with("run-")) {
            kept.push_str(&format!("{RUN_ID}{id}"));
            if let Some(state) = word_after(message, FINISHED) {
                kept.push_str(&format!(" {FINISHED}{state}"));
            }
        } else if message.contains(EXITED) && message.contains(WITHOUT_OUTCOME) {
            kept.push_str(&format!("{EXITED} {WITHOUT_OUTCOME}"));
        }
        return format!("{PLUGIN_MESSAGE_PREFIX}{kept}");
    }
    if text.starts_with("<task-notification>") {
        return "<task-notification>".to_string();
    }
    if let Some((name, _)) = text
        .strip_prefix("The ")
        .and_then(|rest| rest.split_once(" plugin sent a message:"))
        .filter(|(name, _)| !name.is_empty() && !name.contains('\n'))
    {
        return format!("The {name} plugin sent a message:");
    }
    "<text>".to_string()
}

/// A tool result's body, kept only as far as a relais start reads it.
fn result_stub(part: &Value) -> &'static str {
    let text = match &part["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    if text
        .trim_start()
        .starts_with(relais_messages::STARTED_PREFIX)
    {
        relais_messages::STARTED_PREFIX
    } else {
        ""
    }
}

/// The fixture's text: its members, then one entry per line.
fn render(head: &Map<String, Value>, entries: &[Value]) -> String {
    let mut text = String::from("{\n");
    for (key, value) in head {
        text.push_str(&format!("  {}: {value},\n", json!(key)));
    }
    text.push_str("  \"entries\": [");
    for (at, entry) in entries.iter().enumerate() {
        text.push_str(if at == 0 { "\n    " } else { ",\n    " });
        text.push_str(&entry.to_string());
    }
    if !entries.is_empty() {
        text.push_str("\n  ");
    }
    text.push_str("]\n}\n");
    text
}

/// Whether the person's next turn after the Stop at byte `end` ran work: an
/// Edit, a Write, a NotebookEdit, a `git worktree add` or a relais run.
fn early_after(path: &Path, end: u64) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(end))?;
    let mut turn: Vec<Value> = Vec::new();
    let mut started = false;
    for line in BufReader::new(file).split(b'\n') {
        let Ok(entry) = serde_json::from_slice::<Value>(&line?) else {
            continue; // a line that is not JSON is no entry
        };
        if is_person_prompt(&entry) {
            if started {
                break;
            }
            started = true;
        } else if started {
            turn.push(entry);
        }
    }
    Ok(work_in(&turn))
}

/// Export the Stops the journal records.
pub fn export(options: &Options) -> Report {
    let mut exporter = Exporter {
        options,
        rows: read_rows(&options.journal),
        scrub: Scrub {
            home: options.home.clone(),
        },
        indexes: HashMap::new(),
        after_hold: HashMap::new(),
        last_placed: None,
    };
    let mut targets: Vec<usize> = (0..exporter.rows.len())
        .filter(|at| exporter.rows[*at].kind.is_some())
        .collect();
    if let Some(sample) = options.sample {
        let drop = targets.len().saturating_sub(sample);
        targets.drain(..drop);
    }
    let mut report = Report {
        dir: options.out.clone(),
        candidates: targets.len(),
        ..Report::default()
    };
    let known: HashSet<String> = corpus::parse(&options.cases_text)
        .iter()
        .filter_map(|case| corpus::StopCase::parse(&case.command))
        .filter_map(Result::ok)
        .map(|stop| stop.name)
        .collect();
    let mut seen = HashSet::new();
    let mut case_lines: Vec<String> = Vec::new();
    for target in targets {
        let fixture = match exporter.build(target) {
            Built::Skipped(why) => {
                report.skipped += 1;
                report.notes.push(format!("skipped: {why}"));
                continue;
            }
            Built::Fixture(fixture) => *fixture,
        };
        if !seen.insert(fixture.name.clone()) {
            report.skipped += 1;
            report.notes.push(format!(
                "skipped: {} appears twice in the journal",
                fixture.name
            ));
            continue;
        }
        let path = options.out.join(format!("{}.json", fixture.name));
        let fresh = match std::fs::read_to_string(&path) {
            Ok(existing) if existing == fixture.text => false,
            Ok(_) => {
                report.skipped += 1;
                report.notes.push(format!(
                    "skipped: {} exists with other content",
                    path.display()
                ));
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => {
                report.skipped += 1;
                report
                    .notes
                    .push(format!("skipped: {}: {error}", path.display()));
                continue;
            }
        };
        if fresh {
            let written = std::fs::create_dir_all(&options.out)
                .and_then(|()| write_atomic(&path, &fixture.text));
            if let Err(error) = written {
                report.skipped += 1;
                report
                    .notes
                    .push(format!("skipped: {}: {error}", path.display()));
                continue;
            }
            report.written += 1;
            report.excluded += usize::from(fixture.excluded.is_some());
            report.replay_differs += usize::from(fixture.replay_differs.is_some());
        } else {
            report.already += 1;
        }
        if known.contains(&fixture.name) {
            continue;
        }
        match (&fixture.excluded, &fixture.replay_differs) {
            (Some(why), _) => {
                if fresh {
                    case_lines.push(format!("# excluded: Stop {}: {why}", fixture.name));
                }
            }
            (None, differs) => {
                if let Some(why) = differs {
                    case_lines.push(format!("# replay differs: {why}"));
                }
                case_lines.push(
                    corpus::line_for(
                        corpus::Verdict::Unreviewed,
                        &format!("Stop {}", fixture.name),
                    )
                    .trim_end()
                    .to_string(),
                );
            }
        }
    }
    report.stdout = case_lines;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clear_boundary::judge_case;

    const SESSION: &str = "abcd1234-0000-4000-8000-000000000000";

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "greenbelay-export-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory); // a directory left by a killed run, if any
        std::fs::create_dir_all(&directory).expect("scratch directory");
        directory
    }

    fn timed(mut entry: Value, uuid: &str, seconds: u64) -> Value {
        entry["uuid"] = json!(uuid);
        entry["timestamp"] = json!(format!("2026-11-18T10:00:{seconds:02}.000Z"));
        entry["cwd"] = json!("/Users/someone/work");
        entry
    }

    fn prompt(uuid: &str, text: &str) -> Value {
        timed(
            json!({"type": "user", "message": {"role": "user", "content": text}}),
            uuid,
            1,
        )
    }

    fn assistant(uuid: &str, tokens: u64, blocks: Value) -> Value {
        timed(
            json!({"type": "assistant", "message": {"role": "assistant",
                "usage": {"input_tokens": tokens, "cache_creation_input_tokens": 0,
                          "cache_read_input_tokens": 0, "service_tier": "standard"},
                "content": blocks}}),
            uuid,
            2,
        )
    }

    fn bash(uuid: &str, command: &str) -> Value {
        assistant(
            uuid,
            250_000,
            json!([
                {"type": "text", "text": "SECRET PROSE the assistant wrote"},
                {"type": "tool_use", "id": format!("t-{uuid}"), "name": "Bash",
                 "input": {"command": command, "description": "more prose"}}
            ]),
        )
    }

    fn tool_use(uuid: &str, name: &str, input: Value) -> Value {
        assistant(
            uuid,
            250_000,
            json!([{"type": "tool_use", "id": format!("t-{uuid}"), "name": name, "input": input}]),
        )
    }

    fn tool_result(uuid: &str, id: &str, text: &str) -> Value {
        timed(
            json!({"type": "user", "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": id, "content": text}]}}),
            uuid,
            3,
        )
    }

    fn plugin_message(uuid: &str, text: &str) -> Value {
        prompt(uuid, &format!("The relais plugin sent a message:\n{text}"))
    }

    /// A world: a transcript for `SESSION`, a journal and an output directory.
    struct World {
        root: PathBuf,
        options: Options,
    }

    impl World {
        fn new(name: &str, entries: &[Value], journal_lines: &[String]) -> World {
            let root = scratch(name);
            let transcript = root.join(format!("{SESSION}.jsonl"));
            let body: String = entries.iter().map(|entry| format!("{entry}\n")).collect();
            std::fs::write(&transcript, body).expect("transcript");
            let journal = root.join("journal.log");
            std::fs::write(&journal, journal_lines.join("\n") + "\n").expect("journal");
            let mut options = Options::new(root.join("out"));
            options.journal = vec![journal];
            options.transcripts = vec![transcript];
            options.home = Some("/Users/someone".to_string());
            World { root, options }
        }

        fn fixture(&self, name: &str) -> Value {
            let text = std::fs::read_to_string(self.options.out.join(format!("{name}.json")))
                .expect("fixture written");
            serde_json::from_str(&text).expect("fixture is JSON")
        }

        fn text(&self, name: &str) -> String {
            std::fs::read_to_string(self.options.out.join(format!("{name}.json")))
                .expect("fixture written")
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root); // scratch
        }
    }

    fn line(ts: u64, outcome: &str, excerpt: &str) -> String {
        let stance = if outcome == "denied" {
            "deny"
        } else {
            "advise"
        };
        format!("F {ts} clear-at-boundary {stance} {outcome} abcd1234 work bypassPermissions arm=live {excerpt}")
    }

    fn boundary_turn() -> Vec<Value> {
        vec![
            prompt("p1", "a person's private words"),
            bash("b1", "git worktree remove /Users/someone/wt"),
        ]
    }

    #[test]
    fn an_advice_becomes_a_countable_fixture_with_no_prose_and_no_home() {
        let mut entries = boundary_turn();
        entries.insert(
            1,
            tool_result(
                "r0",
                "t-earlier",
                "an ordinary result body the rule never reads",
            ),
        );
        entries.insert(
            1,
            timed(
                json!({"type": "user", "isMeta": true,
                       "message": {"role": "user", "content": "meta words /Users/someone/x"}}),
                "m1",
                2,
            ),
        );
        let world = World::new(
            "advice",
            &entries,
            &[line(1_794_960_484, "advised", "worktree remove uuid=b1")],
        );
        let report = export(&world.options);
        assert_eq!((report.written, report.skipped, report.excluded), (1, 0, 0));
        assert_eq!(report.stdout, ["?\tStop abcd1234-1794960484"]);
        let text = world.text("abcd1234-1794960484");
        for forbidden in [
            "/Users/",
            "SECRET PROSE",
            "private words",
            "ordinary result body",
            "more prose",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} leaked into {text}");
        }
        assert_eq!(judge_case(&text), Ok(true));
        let fixture = world.fixture("abcd1234-1794960484");
        assert_eq!(fixture["countable"], json!(true));
        assert_eq!(fixture["payload"]["cwd"], json!("~/work"));
        assert_eq!(fixture["state"]["advices"], json!(0));
        assert_eq!(fixture["early"], json!(false));
    }

    #[test]
    fn a_relais_start_outside_the_live_window_replays_the_same() {
        let mut entries = vec![
            prompt("p0", "start"),
            tool_use("s0", "mcp__relais__run", json!({"plan": "x"})),
            tool_result("s1", "t-s0", "Started relais run run-old-1"),
        ];
        for at in 0..40 {
            entries.push(assistant(
                &format!("pad{at}"),
                10,
                json!([{"type": "text", "text": "x".repeat(200)}]),
            ));
        }
        entries.extend(boundary_turn());
        let mut world = World::new(
            "outside",
            &entries,
            &[line(1_794_960_484, "advised", "worktree remove uuid=b1")],
        );
        world.options.first_chunk = 1024;
        world.options.cap = 1 << 20;
        let report = export(&world.options);
        assert_eq!(
            (report.written, report.excluded, report.replay_differs),
            (1, 0, 0)
        );
        let text = world.text("abcd1234-1794960484");
        assert!(
            !text.contains("Started relais run"),
            "the start is outside the window"
        );
        assert_eq!(judge_case(&text), Ok(true));
        assert_eq!(
            world.fixture("abcd1234-1794960484")["source"]["window"]["capped"],
            json!(false)
        );
    }

    #[test]
    fn a_relais_start_inside_the_window_holds_the_boundary() {
        let mut entries = vec![prompt("p0", "start")];
        entries.push(bash("b1", "git worktree remove /tmp/wt"));
        entries.push(tool_use("s0", "mcp__relais__run", json!({"plan": "x"})));
        entries.push(tool_result("s1", "t-s0", "Started relais run run-1"));
        let world = World::new(
            "inside",
            &entries,
            &[line(
                1_794_960_484,
                "watched",
                "pending relais pending 1 uuid=s1",
            )],
        );
        let report = export(&world.options);
        assert_eq!(
            (report.written, report.excluded, report.replay_differs),
            (1, 0, 0)
        );
        let text = world.text("abcd1234-1794960484");
        assert!(text.contains("Started relais run"));
        assert_eq!(judge_case(&text), Ok(false));
    }

    #[test]
    fn a_paired_relais_start_and_result_and_a_decision_replay_the_same() {
        let finished = plugin_message(
            "m1",
            "relais run run-1 finished: needs_decision\nsecret detail",
        );
        let base = vec![
            prompt("p0", "go"),
            tool_use("s0", "mcp__relais__run", json!({"plan": "x"})),
            tool_result("s1", "t-s0", "Started relais run run-1\nmore"),
            finished,
        ];
        let mut undecided = base.clone();
        undecided.push(bash("b1", "git worktree remove /tmp/wt"));
        let world = World::new(
            "undecided",
            &undecided,
            &[line(
                1_794_960_484,
                "watched",
                "pending relais run-1 needs_decision uuid=b1",
            )],
        );
        let report = export(&world.options);
        assert_eq!(
            (report.written, report.excluded, report.replay_differs),
            (1, 0, 0)
        );
        assert!(!world.text("abcd1234-1794960484").contains("secret detail"));

        let mut decided = base;
        decided.push(tool_use(
            "d0",
            "mcp__relais__decide",
            json!({"run": "run-1", "verdict": "x"}),
        ));
        decided.push(bash("b1", "git worktree remove /tmp/wt"));
        let world = World::new(
            "decided",
            &decided,
            &[line(1_794_960_484, "advised", "worktree remove uuid=b1")],
        );
        let report = export(&world.options);
        assert_eq!(
            (report.written, report.excluded, report.replay_differs),
            (1, 0, 0)
        );
        assert_eq!(judge_case(&world.text("abcd1234-1794960484")), Ok(true));
    }

    #[test]
    fn a_pending_uuid_that_needs_the_window_to_grow_is_found() {
        let mut entries = vec![
            prompt("p0", "go"),
            bash("b1", "git worktree remove /tmp/wt"),
        ];
        entries.push(assistant(
            "u1",
            250_000,
            json!([{"type": "text", "text": "waiting"}]),
        ));
        for at in 0..60 {
            entries.push(assistant(
                &format!("pad{at}"),
                10,
                json!([{"type": "text", "text": "y".repeat(200)}]),
            ));
        }
        entries.push(prompt("n1", "<task-notification>\nbody"));
        entries.push(tool_use(
            "w1",
            "Edit",
            json!({"file_path": "/Users/someone/f"}),
        ));
        entries.push(assistant(
            "u2",
            250_000,
            json!([{"type": "text", "text": "done"}]),
        ));
        let mut world = World::new(
            "grow",
            &entries,
            &[
                line(1_794_960_484, "watched", "pending background uuid=u1"),
                line(1_794_960_490, "watched", "pending-work uuid=u2"),
            ],
        );
        world.options.first_chunk = 2048;
        world.options.cap = 1 << 20;
        let report = export(&world.options);
        assert_eq!(
            (report.written, report.excluded, report.replay_differs),
            (2, 0, 0),
            "{:?}",
            report.notes
        );
        let fixture = world.fixture("abcd1234-1794960490");
        assert_eq!(fixture["state"]["pending"]["uuid"], json!("b1"));
        let text = world.text("abcd1234-1794960490");
        assert!(
            text.contains("\"uuid\":\"b1\""),
            "the window grew to the pending entry"
        );
        assert_eq!(judge_case(&text), Ok(false));
        let hold = world.fixture("abcd1234-1794960484");
        assert_eq!(hold["inferred"], json!(true));
        assert_eq!(hold["payload"]["background_busy"], json!(true));
    }

    #[test]
    fn a_capped_tail_reproduces_the_live_window() {
        let mut entries = vec![prompt("p0", "go")];
        for at in 0..30 {
            entries.push(assistant(
                &format!("pad{at}"),
                10,
                json!([{"type": "text", "text": "z".repeat(100)}]),
            ));
        }
        entries.push(prompt("p1", "again"));
        entries.push(bash("b1", "git worktree remove /tmp/wt"));
        let mut world = World::new(
            "capped",
            &entries,
            &[line(
                1_794_960_484,
                "advised",
                "worktree remove tail-capped uuid=b1",
            )],
        );
        world.options.first_chunk = 2048;
        world.options.cap = 2048;
        let report = export(&world.options);
        assert_eq!(
            (report.written, report.excluded),
            (1, 0),
            "{:?}",
            report.stdout
        );
        let fixture = world.fixture("abcd1234-1794960484");
        assert_eq!(fixture["source"]["window"]["capped"], json!(true));
        assert_eq!(judge_case(&world.text("abcd1234-1794960484")), Ok(true));

        let journal_without_cap = [line(1_794_960_484, "advised", "worktree remove uuid=b1")];
        std::fs::write(
            &world.options.journal[0],
            journal_without_cap.join("\n") + "\n",
        )
        .unwrap();
        std::fs::remove_dir_all(&world.options.out).unwrap();
        let report = export(&world.options);
        assert_eq!(report.excluded, 1, "the capped flag must match the journal");
        assert!(report.stdout[0]
            .starts_with("# excluded: Stop abcd1234-1794960484: the rebuilt window is capped"));
    }

    #[test]
    fn a_stop_with_no_uuid_is_found_by_its_second() {
        let mut entries = boundary_turn();
        entries.push(timed(
            json!({"type": "assistant", "message": {"content": []}}),
            "late",
            59,
        ));
        let world = World::new(
            "by-time",
            &entries,
            &[line(1_794_960_484, "advised", "worktree remove")],
        );
        // 2026-11-18T10:00:02Z is 1_794_996_002; a journal second before it
        // finds no entry at or before it.
        let report = export(&world.options);
        assert_eq!(report.excluded, 1);
        let mut later = World::new(
            "by-time-later",
            &entries,
            &[line(1_794_996_002, "advised", "worktree remove")],
        );
        later.options.out = later.root.join("out");
        let report = export(&later.options);
        assert_eq!(
            (report.written, report.excluded),
            (1, 0),
            "{:?}",
            report.stdout
        );
        let fixture = later.fixture("abcd1234-1794996002");
        assert_eq!(fixture["source"]["stop_uuid"], json!("b1"));
    }

    #[test]
    fn the_edit_in_the_next_person_turn_sets_early() {
        let mut entries = boundary_turn();
        entries.push(prompt("p2", "next"));
        entries.push(tool_use("e1", "Edit", json!({"file_path": "/x"})));
        let world = World::new(
            "early",
            &entries,
            &[line(1_794_960_484, "advised", "worktree remove uuid=b1")],
        );
        export(&world.options);
        assert_eq!(world.fixture("abcd1234-1794960484")["early"], json!(true));
    }

    #[test]
    fn a_replay_that_disagrees_with_the_journal_is_commented() {
        let entries = boundary_turn();
        let world = World::new(
            "differs",
            &entries,
            &[line(1_794_960_484, "watched", "pending-work uuid=b1")],
        );
        let report = export(&world.options);
        assert_eq!(
            report.excluded, 1,
            "pending-work with no hold to rebuild it from"
        );

        let world = World::new(
            "differs2",
            &entries,
            &[line(
                1_794_960_484,
                "watched",
                "pending relais pending 1 uuid=b1",
            )],
        );
        let report = export(&world.options);
        assert_eq!((report.written, report.replay_differs), (1, 1));
        assert_eq!(report.stdout.len(), 2);
        assert!(report.stdout[0].starts_with("# replay differs: journal says pending relais"));
        assert_eq!(report.stdout[1], "?\tStop abcd1234-1794960484");
    }

    #[test]
    fn a_second_export_writes_nothing_and_never_overwrites() {
        let world = World::new(
            "again",
            &boundary_turn(),
            &[line(1_794_960_484, "advised", "worktree remove uuid=b1")],
        );
        let first = export(&world.options);
        assert_eq!(first.written, 1);
        let mut options = Options::new(world.options.out.clone());
        options.journal = world.options.journal.clone();
        options.transcripts = world.options.transcripts.clone();
        options.home = world.options.home.clone();
        options.cases_text = "?\tStop abcd1234-1794960484\n".to_string();
        let second = export(&options);
        assert_eq!((second.written, second.already, second.skipped), (0, 1, 0));
        assert!(second.stdout.is_empty());
        assert!(!second.failed());

        let path = world.options.out.join("abcd1234-1794960484.json");
        std::fs::write(&path, "{}").unwrap();
        options.cases_text.clear();
        let third = export(&options);
        assert_eq!((third.written, third.skipped), (0, 1));
        assert!(third.failed());
        assert!(third.notes[0].contains("other content"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
    }

    #[test]
    fn a_missing_or_ambiguous_transcript_is_skipped() {
        let mut world = World::new(
            "missing",
            &boundary_turn(),
            &[line(1_794_960_484, "advised", "worktree remove uuid=b1")],
        );
        world.options.transcripts.clear();
        let report = export(&world.options);
        assert_eq!((report.skipped, report.written), (1, 0));
        assert!(report.failed());
        world.options.transcripts = vec![
            world.root.join(format!("{SESSION}.jsonl")),
            world.root.join("abcd1234-ffff.jsonl"),
        ];
        let report = export(&world.options);
        assert!(
            report.notes[0].contains("matches 2 transcripts"),
            "{:?}",
            report.notes
        );
    }

    #[test]
    fn a_fixture_that_is_not_countable_is_an_error_for_the_judge() {
        let world = World::new(
            "uncountable",
            &boundary_turn(),
            &[line(
                1_794_960_484,
                "advised",
                "worktree remove uuid=nowhere",
            )],
        );
        let report = export(&world.options);
        assert_eq!(report.excluded, 1);
        let result = judge_case(&world.text("abcd1234-1794960484"));
        assert_eq!(
            result,
            Err("not countable: no entry uuid=nowhere in the transcript".to_string())
        );
        assert!(report.stdout[0].starts_with("# excluded: Stop abcd1234-1794960484: "));
        assert_eq!(report.stdout.len(), 1, "an excluded Stop has no case line");
    }

    #[test]
    fn the_sample_keeps_the_most_recent_stops() {
        let world = World::new(
            "sample",
            &boundary_turn(),
            &[
                line(1_794_960_480, "advised", "worktree remove uuid=b1"),
                line(1_794_960_490, "advised", "worktree remove uuid=b1"),
            ],
        );
        let mut options = Options::new(world.options.out.clone());
        options.journal = world.options.journal.clone();
        options.transcripts = world.options.transcripts.clone();
        options.sample = Some(1);
        let report = export(&options);
        assert_eq!(report.written, 1);
        assert!(world.options.out.join("abcd1234-1794960490.json").exists());
    }

    #[test]
    fn home_and_other_users_become_a_tilde() {
        let scrub = Scrub {
            home: Some("/home/me".to_string()),
        };
        assert_eq!(
            scrub.text("cd /home/me/x && ls /Users/other/y"),
            "cd ~/x && ls ~/y"
        );
        assert_eq!(scrub.text("-C '/Users/me'"), "-C '~'");
    }
}
