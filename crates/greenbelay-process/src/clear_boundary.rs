//! The `clear-at-boundary` judgement of a `Stop`: what a transcript tail says
//! about the context size and the current turn, whether the turn reached a
//! boundary, whether a relais run still waits on this session, and whether
//! the repositories the turn touched are clean. The reading of the tail and
//! of relais messages is pure; [`on_stop`] is the shell around it, which
//! spawns git, keeps the session's state file and journals each outcome.
//! `src/hook.rs` orders it against `plan-phases-open`;
//! `docs/plans/2026-11-13-advise-clear-at-a-boundary.md` is the
//! specification.
//!
//! greenbelay never spawns `relais` (ADR-0064): the run state is read from
//! what the transcript's own relais tool results already showed, through
//! the phrases in [`relais_messages`].

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use greenbelay_guard::atomic::write_atomic;
use greenbelay_guard::corpus::FixtureError;
use greenbelay_guard::decision::{self, Decision};
use greenbelay_guard::git::{bytes_bounded_detail, BoundedFailure};
use greenbelay_guard::journal;
use greenbelay_guard::payload::Stop;
use greenbelay_guard::rules::clear_at_boundary::RULE;
use greenbelay_guard::rules::Stance;
use serde::Deserialize;
use serde_json::Value;

use crate::plan_phases::PhaseFact;

pub use greenbelay_guard::rules::clear_at_boundary::MARKER;

/// The context size, in tokens, past which a clear pays for itself.
///
/// Break-even: a fresh session re-reads its prefix R once at the 5-minute
/// cache write price, 25 times a cache read on Opus 5.5 (`25·R`), while the
/// session kept going pays a read `R` less than a read of the bigger context
/// `S` each later turn (`S − R`). Clearing wins after `k = 25·R/(S − R)`
/// further turns. With the prefix R of about 100k tokens, S = 200k gives
/// `k = 25`: a session expected to run 25 more turns repays the clear, and
/// every larger S repays it sooner.
pub const SIZE: u64 = 200_000;

/// The most advices one session gets; from the next boundary on the rule is
/// silent (`watched cap`). Backtest: 789 advices, 44 sessions over 3, at
/// most 54 in one; a cap of 3 leaves 174 advices and
/// keeps 30 of 31 compacted sessions advised before their first compaction.
pub const MAX_ADVICES_PER_SESSION: u32 = 3;

/// Home as fixtures write it: the exporter scrubs `$HOME` to `~`, and a
/// replay resolves a boundary's `~` against this literal, so a fixture's
/// per-directory keys read as the scrubbed paths.
pub(crate) const SYMBOLIC_HOME: &str = "~";

/// The first chunk of a transcript tail.
pub const TAIL_FIRST_CHUNK: u64 = 64 * 1024;
/// The largest tail read.
// holds-until: a session's relais history fits in 8 MB of transcript; past
// that, a run index kept in the state file instead of the tail.
pub const TAIL_CAP: u64 = 8 * 1024 * 1024;

/// The entries of a transcript's tail, oldest first.
#[derive(Debug)]
pub struct Tail {
    pub entries: Vec<Value>,
    /// The cap was reached before the start of the file: older entries exist
    /// that were not read (the hook journals `tail-capped`).
    pub capped: bool,
    /// The byte offset of the first entry read (0 when the file start was
    /// reached).
    pub start: u64,
    /// The byte offset the read ended at.
    pub end: u64,
}

/// [`read_tail_until`] with the defaults, stopping once both the context size
/// and the turn window are in the tail.
pub fn read_tail(path: &Path) -> std::io::Result<Tail> {
    read_tail_until(path, TAIL_FIRST_CHUNK, TAIL_CAP, |entries| {
        context_size(entries).is_some() && turn_window(entries).is_some()
    })
}

/// Read a JSONL file backwards in chunks of `first_chunk` bytes, doubling up
/// to `cap`, until `enough` accepts the entries read, the file start is
/// reached or the cap is.
///
/// A chunk that starts inside the file begins with a cut line, which is
/// dropped; a line that is not JSON is skipped.
pub fn read_tail_until(
    path: &Path,
    first_chunk: u64,
    cap: u64,
    enough: impl Fn(&[Value]) -> bool,
) -> std::io::Result<Tail> {
    let size = std::fs::metadata(path)?.len();
    read_tail_until_at(path, size, first_chunk, cap, enough)
}

/// [`read_tail_until`] over the file as it stood at byte offset `end`: the
/// bytes after `end` are never read, so a replay sees the window a Stop
/// saw. An `end` past the file's size is the size.
pub fn read_tail_until_at(
    path: &Path,
    end: u64,
    first_chunk: u64,
    cap: u64,
    enough: impl Fn(&[Value]) -> bool,
) -> std::io::Result<Tail> {
    let mut file = std::fs::File::open(path)?;
    let size = end.min(file.metadata()?.len());
    let mut chunk = first_chunk.clamp(1, cap.max(1));
    loop {
        let start = size.saturating_sub(chunk);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        (&mut file).take(size - start).read_to_end(&mut bytes)?;
        let whole_lines = if start > 0 {
            without_first_line(&bytes)
        } else {
            &bytes
        };
        let entries = parse_lines(whole_lines);
        if start == 0 {
            return Ok(Tail {
                entries,
                capped: false,
                start: 0,
                end: size,
            });
        }
        let first_whole_line = start + (bytes.len() - whole_lines.len()) as u64;
        if chunk >= cap {
            return Ok(Tail {
                entries,
                capped: true,
                start: first_whole_line,
                end: size,
            });
        }
        if enough(&entries) {
            return Ok(Tail {
                entries,
                capped: false,
                start: first_whole_line,
                end: size,
            });
        }
        chunk = chunk.saturating_mul(2).min(cap);
    }
}

/// The bytes after the first newline: a chunk read from inside the file
/// starts in the middle of a line.
fn without_first_line(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(&[], |at| &bytes[at + 1..])
}

fn parse_lines(bytes: &[u8]) -> Vec<Value> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect()
}

pub(crate) fn is_sidechain(entry: &Value) -> bool {
    entry["isSidechain"].as_bool() == Some(true)
}

pub(crate) fn entry_type(entry: &Value) -> &str {
    entry["type"].as_str().unwrap_or("")
}

/// The content blocks of an entry's message; a plain string is no blocks.
pub(crate) fn blocks(entry: &Value) -> &[Value] {
    entry
        .pointer("/message/content")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Input, cache-creation and cache-read tokens of the last main-chain
/// assistant entry that carries `message.usage`.
pub fn context_size(entries: &[Value]) -> Option<u64> {
    entries
        .iter()
        .rev()
        .filter(|entry| entry_type(entry) == "assistant" && !is_sidechain(entry))
        .find_map(|entry| {
            let usage = entry.pointer("/message/usage")?.as_object()?;
            let tokens = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
            Some(
                tokens("input_tokens")
                    + tokens("cache_creation_input_tokens")
                    + tokens("cache_read_input_tokens"),
            )
        })
}

/// The entries after the last person prompt.
#[derive(Debug)]
pub struct TurnWindow<'a> {
    /// The prompt's `uuid`, empty when the entry carries none.
    pub prompt_uuid: String,
    pub entries: &'a [Value],
}

/// A `user` entry that is a person's prompt and not a tool result or a
/// harness-injected message.
fn is_prompt(entry: &Value) -> bool {
    if entry_type(entry) != "user" || is_sidechain(entry) || entry["isMeta"].as_bool() == Some(true)
    {
        return false;
    }
    match entry.pointer("/message/content") {
        Some(Value::String(_)) => true,
        Some(Value::Array(parts)) => !parts
            .iter()
            .any(|part| part["type"].as_str() == Some("tool_result")),
        _ => false,
    }
}

/// The text of a user entry's prompt: a plain string or the first text block.
fn prompt_text(entry: &Value) -> Option<&str> {
    match entry.pointer("/message/content")? {
        Value::String(text) => Some(text),
        Value::Array(parts) => parts.iter().find_map(|part| part["text"].as_str()),
        _ => None,
    }
}

/// A prompt the harness injected: a task notification, or a message the
/// engine framed as `The <name> plugin sent a message:`.
pub fn is_injected(entry: &Value) -> bool {
    if entry_type(entry) != "user" {
        return false;
    }
    prompt_text(entry).is_some_and(|text| {
        text.starts_with("<task-notification>")
            || text
                .strip_prefix("The ")
                .and_then(|rest| rest.split_once(" plugin sent a message:"))
                .is_some_and(|(name, _)| !name.is_empty() && !name.contains('\n'))
    })
}

/// A prompt a person typed: a prompt that is not injected.
pub fn is_person_prompt(entry: &Value) -> bool {
    is_prompt(entry) && !is_injected(entry)
}

/// `None` when no prompt is within the entries read (the caller journals
/// `turn-too-long`). A turn opens at the last prompt of any kind: relais
/// plugin messages and task notifications start a turn too.
pub fn turn_window(entries: &[Value]) -> Option<TurnWindow<'_>> {
    let at = entries.iter().rposition(is_prompt)?;
    Some(TurnWindow {
        prompt_uuid: entries[at]["uuid"].as_str().unwrap_or("").to_string(),
        entries: &entries[at + 1..],
    })
}

/// The shape that keeps the walk from reading the directory a command acted
/// on. Its name goes to the journal row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Subshell,
    CommandSubst,
    CdDash,
    CdPiped,
    CdOr,
    Keyword,
    LoopVar,
    Export,
    Unassigned,
    NotLiteral,
    Conditional,
}

impl Shape {
    /// The shape's name as the journal's `unresolved <shape> <what>` row
    /// writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Subshell => "subshell",
            Self::CommandSubst => "command-subst",
            Self::CdDash => "cd-dash",
            Self::CdPiped => "cd-piped",
            Self::CdOr => "cd-or",
            Self::Keyword => "keyword",
            Self::LoopVar => "loop-var",
            Self::Export => "export",
            Self::Unassigned => "unassigned",
            Self::NotLiteral => "not-literal",
            Self::Conditional => "conditional",
        }
    }
}

/// A boundary the turn reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Boundary {
    /// What it was, in a few words.
    pub what: String,
    /// The `uuid` of the assistant entry whose Bash call it was found in,
    /// empty when the entry carries none.
    pub uuid: String,
    /// The Bash command it was found in.
    pub command: String,
    /// The `-C` chain of the git call, resolved against the payload cwd;
    /// empty when the call had none.
    pub dirs: Vec<PathBuf>,
    /// The shape that kept the walk from reading the directory the command
    /// acted on; `None` when it read it.
    pub unresolved: Option<Shape>,
}

/// A boundary command's name, the directories it acted on, and the shape
/// that kept the walk from reading them.
type Reached = (&'static str, Vec<PathBuf>, Option<Shape>);

/// The boundary commands among the Bash tool calls of the window. `home`
/// resolves a `-C` directory written from `~` or `$HOME`. A call's commands
/// are walked in order from the `cwd` of the entry carrying it (the payload
/// `cwd` when the entry has none), so a chained `cd` and a `NAME=` set in the
/// same command choose the directory a boundary acted on.
pub fn boundaries(window: &TurnWindow<'_>, cwd: &Path, home: Option<&Path>) -> Vec<Boundary> {
    let mut found = Vec::new();
    for entry in window
        .entries
        .iter()
        .filter(|entry| entry_type(entry) == "assistant" && !is_sidechain(entry))
    {
        let start = entry_start(entry["cwd"].as_str(), cwd, home);
        for block in blocks(entry) {
            if block["type"].as_str() != Some("tool_use") || block["name"].as_str() != Some("Bash")
            {
                continue;
            }
            let Some(command) = block.pointer("/input/command").and_then(Value::as_str) else {
                continue;
            };
            let mut walk = Walk::new(start.clone());
            let steps = walk_steps(command);
            for (at, step) in steps.iter().enumerate() {
                if let Some((what, dirs, unresolved)) =
                    walk.step(step, steps.get(at + 1), cwd, home)
                {
                    found.push(Boundary {
                        what: what.to_string(),
                        uuid: entry["uuid"].as_str().unwrap_or("").to_string(),
                        command: command.to_string(),
                        dirs,
                        unresolved,
                    });
                }
            }
        }
    }
    found
}

/// Where a Bash call began: the entry's own `cwd` when it is a non-empty
/// rooted path (in a replay, also a path under the symbolic home, which is
/// relative), else the payload `cwd`.
fn entry_start(entry_cwd: Option<&str>, payload_cwd: &Path, home: Option<&Path>) -> PathBuf {
    // Rooted, not `is_absolute`: on Windows a `/w` path has no drive and is
    // not absolute, yet it names no directory relative to the hook's own.
    let usable = |path: &Path| {
        path.has_root()
            || home.is_some_and(|home| {
                !home.as_os_str().is_empty() && !home.is_absolute() && path.starts_with(home)
            })
    };
    entry_cwd
        .filter(|text| !text.is_empty())
        .map(PathBuf::from)
        .filter(|path| usable(path))
        .unwrap_or_else(|| payload_cwd.to_path_buf())
}

/// How one character of a shell word was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quoting {
    Bare,
    Single,
    Double,
    /// After a backslash, outside quotes or inside double quotes.
    Escaped,
}

/// A shell word with its quotes removed, and how each of its characters
/// was written: a `~` or `$HOME` expands only when written bare (or, for
/// `$HOME`, inside double quotes), which the text alone no longer shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Word {
    text: String,
    /// One per `char` of `text`.
    quoting: Vec<Quoting>,
}

impl Word {
    fn push(&mut self, c: char, quoting: Quoting) {
        self.text.push(c);
        self.quoting.push(quoting);
    }
}

/// What ended a simple command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    /// `;`, a newline or the end of the command.
    Sequence,
    And,
    Or,
    /// `|` and zsh's `|&`.
    Pipe,
    /// A lone `&`.
    Background,
}

/// One simple command, the operator that ended it, and whether it is text
/// that sets no shell state: a heredoc body line, which the shell reads as
/// data.
#[derive(Debug, Clone)]
struct Step {
    words: Vec<Word>,
    end: Operator,
    heredoc_body: bool,
}

/// Split a shell command into simple commands of words, honouring quotes:
/// `;`, `&`, `|` and newlines end a command.
fn segments(command: &str) -> Vec<Vec<String>> {
    walk_steps(command)
        .into_iter()
        .map(|step| step.words.into_iter().map(|word| word.text).collect())
        .collect()
}

/// The heredoc delimiters a simple command opens (`<<EOF`, `<<- 'EOF'`).
fn heredoc_delimiters(words: &[Word]) -> Vec<String> {
    let mut delimiters = Vec::new();
    for (at, word) in words.iter().enumerate() {
        if word.quoting.first() != Some(&Quoting::Bare)
            || !word.text.starts_with("<<")
            || word.text.starts_with("<<<")
        {
            continue;
        }
        let rest = word.text[2..].trim_start_matches('-');
        let delimiter = if rest.is_empty() {
            words.get(at + 1).map(|next| next.text.clone())
        } else {
            Some(rest.to_string())
        };
        delimiters.extend(delimiter);
    }
    delimiters
}

/// The words being read and the steps already read.
#[derive(Default)]
struct StepReader {
    steps: Vec<Step>,
    words: Vec<Word>,
    word: Word,
    in_word: bool,
    delimiters_pending: VecDeque<String>,
    heredoc_delimiter: Option<String>,
}

impl StepReader {
    fn end_word(&mut self) {
        if self.in_word {
            self.words.push(std::mem::take(&mut self.word));
            self.in_word = false;
        }
    }

    fn end_step(&mut self, end: Operator) {
        self.end_word();
        if self.words.is_empty() {
            return;
        }
        let heredoc_body = self.heredoc_delimiter.is_some();
        let words = std::mem::take(&mut self.words);
        if !heredoc_body {
            self.delimiters_pending.extend(heredoc_delimiters(&words));
        }
        self.steps.push(Step {
            words,
            end,
            heredoc_body,
        });
    }

    /// At a newline ending `line`: the delimiter line closes a heredoc body,
    /// and the next pending heredoc's body starts.
    fn end_line(&mut self, line: &str) {
        if self
            .heredoc_delimiter
            .as_deref()
            .is_some_and(|delimiter| line.trim_start_matches('\t') == delimiter)
        {
            self.heredoc_delimiter = None;
        }
        if self.heredoc_delimiter.is_none() {
            self.heredoc_delimiter = self.delimiters_pending.pop_front();
        }
    }
}

/// [`segments`], each word keeping how its characters were quoted, with the
/// operator that ended it.
fn walk_steps(command: &str) -> Vec<Step> {
    let mut reader = StepReader::default();
    let mut chars = command.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let at_line_start = at == 0 || command.as_bytes()[at - 1] == b'\n';
        if reader.heredoc_delimiter.is_some() && at_line_start && c != '\n' {
            let line_end = command[at..].find('\n').map_or(command.len(), |n| at + n);
            for text in command[at..line_end].split_whitespace() {
                reader.in_word = true;
                for raw in text.chars() {
                    reader.word.push(raw, Quoting::Bare);
                }
                reader.end_word();
            }
            reader.end_step(Operator::Sequence);
            while chars.next_if(|&(next_at, _)| next_at < line_end).is_some() {}
            continue;
        }
        match c {
            '#' if !reader.in_word => while chars.next_if(|&(_, next)| next != '\n').is_some() {},
            '\'' => {
                reader.in_word = true;
                for (_, inner) in chars.by_ref() {
                    if inner == '\'' {
                        break;
                    }
                    reader.word.push(inner, Quoting::Single);
                }
            }
            '"' => {
                reader.in_word = true;
                while let Some((_, inner)) = chars.next() {
                    match inner {
                        '"' => break,
                        '\\' => {
                            if let Some((_, next)) = chars.next() {
                                reader.word.push(next, Quoting::Escaped);
                            }
                        }
                        other => reader.word.push(other, Quoting::Double),
                    }
                }
            }
            '\\' => {
                reader.in_word = true;
                if let Some((_, next)) = chars.next() {
                    reader.word.push(next, Quoting::Escaped);
                }
            }
            ';' => reader.end_step(Operator::Sequence),
            '\n' => {
                reader.end_step(Operator::Sequence);
                let line_start = command[..at].rfind('\n').map_or(0, |newline| newline + 1);
                reader.end_line(&command[line_start..at]);
            }
            '&' => {
                let after_redirect = reader.in_word
                    && reader.word.quoting.last() == Some(&Quoting::Bare)
                    && (reader.word.text.ends_with('>') || reader.word.text.ends_with('<'));
                match chars.peek().map(|&(_, next)| next) {
                    Some('&') => {
                        chars.next();
                        reader.end_step(Operator::And);
                    }
                    next if after_redirect || next == Some('>') => {
                        reader.in_word = true;
                        reader.word.push('&', Quoting::Bare);
                    }
                    _ => reader.end_step(Operator::Background),
                }
            }
            '|' => match chars.peek().map(|&(_, next)| next) {
                Some('|') => {
                    chars.next();
                    reader.end_step(Operator::Or);
                }
                Some('&') => {
                    chars.next();
                    reader.end_step(Operator::Pipe);
                }
                _ => reader.end_step(Operator::Pipe),
            },
            c if c.is_whitespace() => reader.end_word(),
            other => {
                reader.in_word = true;
                reader.word.push(other, Quoting::Bare);
            }
        }
    }
    reader.end_step(Operator::Sequence);
    reader.steps
}

/// What follows a leading home reference the shell expands: `~`, `$HOME` or
/// `${HOME}`, then the end of the word or a `/`. The `~` must be bare (POSIX
/// XCU 2.6.1: an unquoted tilde-prefix, ended by an unquoted `/`); the
/// parameter must be bare or inside double quotes, every character of it in
/// the same quoting (2.2.2: single quotes and a backslash keep `$` literal,
/// and `$HO"ME"` is the parameter `HO`). `~user`, `~+`, `~-` and `$HOMEX`
/// are not home; `${HOME}x` is a sibling of home (`/hx`), kept as written
/// like any word this does not resolve. `None` when the word does not start
/// with home.
fn home_suffix(word: &Word) -> Option<&str> {
    let quoting = |at: usize| word.quoting.get(at).copied();
    let length = if word.text.starts_with('~') {
        (quoting(0) == Some(Quoting::Bare)).then_some(1)?
    } else {
        let parameter = ["${HOME}", "$HOME"]
            .into_iter()
            .find(|parameter| word.text.starts_with(parameter))?;
        let first = quoting(0)?;
        let same = (0..parameter.len()).all(|at| quoting(at) == Some(first));
        (same && matches!(first, Quoting::Bare | Quoting::Double)).then_some(parameter.len())?
    };
    // The prefix is ASCII, so its char count is its byte length.
    let rest = &word.text[length..];
    match rest.chars().next() {
        None => Some(rest),
        Some('/') if !word.text.starts_with('~') || quoting(length) == Some(Quoting::Bare) => {
            Some(rest)
        }
        Some(_) => None,
    }
}

/// The home directory a live hook resolves `~` against: `$HOME` when it is
/// an absolute path. Unset, empty or relative, there is none, and a `~` word
/// keeps today's handling.
pub(crate) fn home_from(home_variable: Option<std::ffi::OsString>) -> Option<PathBuf> {
    home_variable
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}

fn base_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// A `git` call: the positions of its global `-C` directories among the
/// words, its subcommand and the words after it.
struct GitCall<'a> {
    directories: Vec<usize>,
    subcommand: &'a str,
    rest: &'a [String],
}

fn git_call(words: &[String]) -> Option<GitCall<'_>> {
    let start = words
        .iter()
        .position(|word| !word.contains('=') || word.starts_with('-'))?;
    if base_name(words.get(start)?) != "git" {
        return None;
    }
    let mut directories = Vec::new();
    let mut at = start + 1;
    while let Some(word) = words.get(at) {
        match word.as_str() {
            "-C" => {
                words.get(at + 1)?;
                directories.push(at + 1);
                at += 2;
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" => at += 2,
            option if option.starts_with('-') => at += 1,
            subcommand => {
                return Some(GitCall {
                    directories,
                    subcommand,
                    rest: &words[at + 1..],
                });
            }
        }
    }
    None
}

/// `base` joined with `directory`, `.` and `..` folded away lexically.
fn resolve(base: &Path, directory: &str) -> PathBuf {
    fold(&base.join(directory))
}

/// `.` and `..` folded away lexically. Above a relative root (the replay's
/// symbolic home `~`), a `..` pops the root itself and leaves an empty
/// path, which the check skips; live, home is absolute and `..` climbs to
/// its parent. Both count as clean unless that parent is a work tree.
fn fold(joined: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    resolved
}

/// Several `-C` flags chain as git does: each relative one against the last.
/// A word that starts with home (see [`home_suffix`]) is absolute once the
/// shell expands it, so it replaces the chain built so far; with no `home`,
/// it is joined as written (the behaviour before home was read).
fn chain(cwd: &Path, home: Option<&Path>, directories: &[&Word]) -> Vec<PathBuf> {
    if directories.is_empty() {
        return Vec::new();
    }
    vec![directories.iter().fold(cwd.to_path_buf(), |base, word| {
        match home.zip(home_suffix(word)) {
            Some((home, suffix)) => fold(&home.join(suffix.trim_start_matches('/'))),
            None => resolve(&base, &word.text),
        }
    })]
}

/// What a shell variable holds at a point of the command.
#[derive(Debug, Clone)]
enum Binding {
    Known(Expanded),
    /// Set to something the walk does not read; the shape says why.
    Unknown(Shape),
}

/// A word with its variables replaced. `rooted` is whether it names an
/// absolute location: it starts with `/`, or with home (which is `~` in a
/// replay).
#[derive(Debug, Clone)]
struct Expanded {
    text: String,
    rooted: bool,
}

impl Expanded {
    fn path(&self, base: Result<&Path, Shape>) -> Result<PathBuf, Shape> {
        if self.rooted || self.text.starts_with('/') {
            Ok(fold(Path::new(&self.text)))
        } else {
            Ok(resolve(base?, &self.text))
        }
    }
}

/// A step that is `exit` or `return`, optionally with a numeric status, and
/// the whole of what follows a `||`.
fn leaves_the_shell(step: &Step) -> bool {
    let ends_alone = matches!(step.end, Operator::Sequence);
    let words = without_redirects(&step.words);
    let status_ok = |status: Option<&Word>| {
        status.is_none_or(|word| {
            !word.text.is_empty() && word.text.chars().all(|c| c.is_ascii_digit())
        })
    };
    ends_alone
        && matches!(
            words.first().and_then(|word| keyword(word)),
            Some("exit" | "return")
        )
        && words.len() <= 2
        && status_ok(words.get(1).copied())
}

/// Whether a simple command certainly ran: after a `&&` or `||` it may not
/// have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    Certain,
    Conditional,
}

/// Whether a simple command runs in its own process, as part of a pipeline or
/// in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piping {
    Alone,
    Piped,
}

/// The shell state a Bash call's simple commands leave behind, in order:
/// the directory it is in and the variables it set. A state the walk cannot
/// read is `Err`/`Unknown` with the shape that says why.
struct Walk {
    start: PathBuf,
    here: Result<PathBuf, Shape>,
    vars: BTreeMap<String, Binding>,
    /// The shape of each compound command (`if`, `for`, `{`…) still open.
    compounds: Vec<Shape>,
    /// Depth of `$(` opened in an earlier simple command and not yet closed.
    substitution_depth: usize,
    previous: Option<Operator>,
    /// Names set after a `&&`/`||` in the current list, each with whether
    /// it keeps its value when the list ends: only a name with no earlier
    /// binding, set after `&&` alone. Had it not run, the name holds what
    /// the environment gave it, which the walk reads as `unassigned`, so
    /// following it only adds a check.
    conditional_names: Vec<(String, bool)>,
    conditional_cd: bool,
}

fn keyword(word: &Word) -> Option<&str> {
    word.quoting
        .iter()
        .all(|quoting| *quoting == Quoting::Bare)
        .then_some(word.text.as_str())
}

/// Reserved words that continue an open compound command before a simple
/// command in the same step.
const CONTINUING_KEYWORDS: [&str; 4] = ["then", "do", "else", "elif"];

/// Reserved words that open a compound command whose body starts in the
/// same step.
const OPENING_KEYWORDS: [&str; 4] = ["{", "if", "while", "until"];

/// A bare word of [`CONTINUING_KEYWORDS`] or [`OPENING_KEYWORDS`].
fn leads_a_command(word: &Word) -> bool {
    keyword(word)
        .is_some_and(|text| CONTINUING_KEYWORDS.contains(&text) || OPENING_KEYWORDS.contains(&text))
}

/// A step's words after the reserved words that lead its simple command,
/// as the walk strips them: `do git worktree remove x` is a `git` call.
fn command_words(words: &[Word]) -> &[Word] {
    let skipped = words
        .iter()
        .take_while(|word| leads_a_command(word))
        .count();
    &words[skipped..]
}

/// The command `command` or `builtin` runs, after their options (`-p`,
/// `-v`, `-V`, `--`) and any nested `command`/`builtin`.
fn wrapped_command<'a>(arguments: &[&'a Word]) -> Option<&'a str> {
    let mut rest = arguments.iter().map(|word| word.text.as_str());
    let mut next = rest.next();
    while let Some(text) = next {
        if text == "--" {
            return rest.next();
        }
        if !(text.len() > 1 && text.starts_with('-')) && text != "command" && text != "builtin" {
            return Some(text);
        }
        next = rest.next();
    }
    None
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `NAME=value`, the name bare, with the value as a word.
fn assignment(word: &Word) -> Option<(&str, Word)> {
    let equals = word.text.find('=')?;
    let name = &word.text[..equals];
    (is_identifier(name) && word.quoting[..equals].iter().all(|q| *q == Quoting::Bare)).then(|| {
        (
            name,
            Word {
                text: word.text[equals + 1..].to_string(),
                quoting: word.quoting[equals + 1..].to_vec(),
            },
        )
    })
}

/// Whether the value holds a command substitution the shell would run.
fn substitutes(assigned: &Word) -> bool {
    let live = |at: usize| {
        assigned
            .quoting
            .get(at)
            .is_some_and(|q| *q != Quoting::Single)
    };
    assigned
        .text
        .char_indices()
        .enumerate()
        .any(|(at, (byte, c))| live(at) && (c == '`' || assigned.text[byte..].starts_with("$(")))
}

/// A redirect word: `Some(true)` when its target is the next word.
fn redirect(word: &Word) -> Option<bool> {
    if word.quoting.first() != Some(&Quoting::Bare) {
        return None;
    }
    let operator = word.text.trim_start_matches(|c: char| c.is_ascii_digit());
    if !(operator.starts_with('<') || operator.starts_with('>') || operator.starts_with("&>")) {
        return None;
    }
    Some(operator.trim_start_matches(['<', '>', '&']).is_empty())
}

fn without_redirects(words: &[Word]) -> Vec<&Word> {
    let mut kept = Vec::new();
    let mut skip_target = false;
    for word in words {
        if skip_target {
            skip_target = false;
        } else if let Some(takes_next) = redirect(word) {
            skip_target = takes_next;
        } else {
            kept.push(word);
        }
    }
    kept
}

impl Walk {
    fn new(start: PathBuf) -> Self {
        Self {
            here: Ok(start.clone()),
            start,
            vars: BTreeMap::new(),
            compounds: Vec::new(),
            substitution_depth: 0,
            previous: None,
            conditional_names: Vec::new(),
            conditional_cd: false,
        }
    }

    /// Classify `step` at the current state, then apply its effect.
    fn step(
        &mut self,
        step: &Step,
        next: Option<&Step>,
        payload_cwd: &Path,
        home: Option<&Path>,
    ) -> Option<Reached> {
        let boundary = classify(&step.words, self, payload_cwd, home);
        if !step.heredoc_body && self.substitution_depth == 0 {
            self.apply(step, next, home);
        }
        self.track_substitutions(&step.words);
        if !matches!(step.end, Operator::And | Operator::Or) {
            self.end_list();
        }
        self.previous = Some(step.end);
        boundary
    }

    fn track_substitutions(&mut self, words: &[Word]) {
        for word in words {
            let chars: Vec<char> = word.text.chars().collect();
            for (at, c) in chars.iter().enumerate() {
                if word.quoting[at] == Quoting::Single {
                    continue;
                }
                if *c == '$' && chars.get(at + 1) == Some(&'(') {
                    self.substitution_depth += 1;
                } else if *c == ')' {
                    self.substitution_depth = self.substitution_depth.saturating_sub(1);
                }
            }
        }
    }

    /// A `&&`/`||` list ended: what its later commands set may not have run.
    fn end_list(&mut self) {
        for (name, keeps) in std::mem::take(&mut self.conditional_names) {
            if !keeps {
                self.vars.insert(name, Binding::Unknown(Shape::Conditional));
            }
        }
        if std::mem::take(&mut self.conditional_cd) && !matches!(self.here, Err(Shape::Subshell)) {
            self.here = Err(Shape::CdOr);
        }
    }

    fn compound_shape(&self) -> Option<Shape> {
        self.compounds.last().copied()
    }

    fn bind(&mut self, name: &str, binding: Binding, reach: Reach) {
        if reach == Reach::Conditional {
            // Read before the assignment writes `vars`.
            let after_and = self.previous == Some(Operator::And);
            match self
                .conditional_names
                .iter_mut()
                .find(|(set, _)| set == name)
            {
                Some((_, keeps)) => *keeps &= after_and,
                None => {
                    let keeps = after_and && !self.vars.contains_key(name);
                    self.conditional_names.push((name.to_string(), keeps));
                }
            }
        }
        self.vars.insert(name.to_string(), binding);
    }

    fn apply(&mut self, step: &Step, next: Option<&Step>, home: Option<&Path>) {
        let reach = if matches!(self.previous, Some(Operator::And | Operator::Or)) {
            Reach::Conditional
        } else {
            Reach::Certain
        };
        let piping = if self.previous == Some(Operator::Pipe)
            || matches!(step.end, Operator::Pipe | Operator::Background)
        {
            Piping::Piped
        } else {
            Piping::Alone
        };
        let mut words = step.words.as_slice();
        while let Some(first) = words.first() {
            match keyword(first) {
                Some(text) if CONTINUING_KEYWORDS.contains(&text) => {}
                Some(text) if OPENING_KEYWORDS.contains(&text) => {
                    self.compounds.push(Shape::Keyword);
                }
                Some("for" | "select") => {
                    self.compounds.push(Shape::LoopVar);
                    if let Some(name) = words.get(1).filter(|word| is_identifier(&word.text)) {
                        self.bind(&name.text, Binding::Unknown(Shape::LoopVar), Reach::Certain);
                    }
                    return;
                }
                Some("case") => {
                    self.compounds.push(Shape::Keyword);
                    return;
                }
                Some("done" | "fi" | "esac" | "}") => {
                    self.compounds.pop();
                }
                _ => break,
            }
            words = &words[1..];
        }
        let Some(opening) = words.first() else { return };
        if opening.quoting.first() == Some(&Quoting::Bare) && opening.text.starts_with('(') {
            self.here = Err(Shape::Subshell);
            return;
        }
        let words = without_redirects(words);
        let leading = words
            .iter()
            .take_while(|word| assignment(word).is_some())
            .count();
        let only_assignments = leading == words.len();
        for (name, value) in words[..leading].iter().filter_map(|word| assignment(word)) {
            let binding = if substitutes(&value) {
                Binding::Unknown(Shape::CommandSubst)
            } else if !only_assignments {
                continue;
            } else if let Some(shape) = self.compound_shape() {
                Binding::Unknown(shape)
            } else if piping == Piping::Piped {
                Binding::Unknown(Shape::NotLiteral)
            } else {
                match self.expand(&value, home) {
                    Ok(expanded) => Binding::Known(expanded),
                    Err(shape) => Binding::Unknown(shape),
                }
            };
            self.bind(name, binding, reach);
        }
        let Some(first) = words.get(leading) else {
            return;
        };
        let words = &words[leading..];
        match keyword(first) {
            Some("export" | "local" | "readonly" | "declare" | "typeset") => {
                for word in &words[1..] {
                    let name = word
                        .text
                        .split_once('=')
                        .map_or(word.text.as_str(), |(name, _)| name);
                    if is_identifier(name) {
                        self.bind(name, Binding::Unknown(Shape::Export), reach);
                    }
                }
            }
            Some("read") => {
                for word in words[1..].iter().filter(|word| is_identifier(&word.text)) {
                    self.bind(&word.text, Binding::Unknown(Shape::LoopVar), reach);
                }
            }
            Some("cd") => {
                // `cd T || exit`: what follows runs only when the `cd` worked.
                let end = if step.end == Operator::Or && next.is_some_and(leaves_the_shell) {
                    Operator::Sequence
                } else {
                    step.end
                };
                self.change_directory(&words[1..], end, (reach, piping), home);
            }
            Some(name @ ("pushd" | "popd" | "builtin" | "command" | "eval" | "source" | ".")) => {
                // `command -v jq` or `builtin echo` leave the directory;
                // only a wrapped command that can move it is unread.
                // A wrapped word the walk cannot read (`command "$c" x`)
                // may be `cd`, so it is unread too.
                let moves = match name {
                    "builtin" | "command" => wrapped_command(&words[1..]).is_some_and(|wrapped| {
                        matches!(wrapped, "cd" | "pushd" | "popd" | "eval" | "source" | ".")
                            || wrapped.contains(['$', '`', '*', '?', '['])
                    }),
                    _ => true,
                };
                if moves && !matches!(self.here, Err(Shape::Subshell)) {
                    self.here = Err(Shape::NotLiteral);
                }
            }
            _ => {}
        }
    }

    fn change_directory(
        &mut self,
        arguments: &[&Word],
        end: Operator,
        (reach, piping): (Reach, Piping),
        home: Option<&Path>,
    ) {
        if matches!(self.here, Err(Shape::Subshell)) {
            return;
        }
        let target = self.cd_target(arguments, home);
        self.here = if let Some(shape) = self.compound_shape() {
            Err(shape)
        } else if piping == Piping::Piped {
            Err(Shape::CdPiped)
        } else if end == Operator::Or {
            Err(Shape::CdOr)
        } else {
            target
        };
        if reach == Reach::Conditional {
            self.conditional_cd = true;
        }
    }

    fn cd_target(&self, arguments: &[&Word], home: Option<&Path>) -> Result<PathBuf, Shape> {
        let word = match arguments {
            [] => return home.map(fold).ok_or(Shape::NotLiteral),
            [word] => word,
            _ => return Err(Shape::NotLiteral),
        };
        if word.text == "-" {
            return Err(Shape::CdDash);
        }
        if word.text.starts_with('-') || word.text.starts_with('+') {
            return Err(Shape::NotLiteral);
        }
        let expanded = self.expand(word, home)?;
        let relative_ok = expanded.text == "."
            || expanded.text == ".."
            || expanded.text.starts_with("./")
            || expanded.text.starts_with("../");
        if !(expanded.rooted || expanded.text.starts_with('/') || relative_ok) {
            return Err(Shape::NotLiteral);
        }
        expanded.path(self.here.as_deref().map_err(|shape| *shape))
    }

    /// A word's text with its leading home or `$NAME` replaced, or the shape
    /// that keeps it from being read. A `$` anywhere else, a backtick, an
    /// unquoted glob character or a `~` that is not home is not read.
    fn expand(&self, word: &Word, home: Option<&Path>) -> Result<Expanded, Shape> {
        let live = |at: usize| word.quoting.get(at).is_some_and(|q| *q != Quoting::Single);
        let bare = |at: usize| word.quoting.get(at) == Some(&Quoting::Bare);
        let (prefix, rest_at) = if let Some(suffix) = home_suffix(word) {
            let rest_at = word.text.len() - suffix.len();
            match home {
                Some(home) => (
                    Some(Expanded {
                        text: home.to_string_lossy().into_owned(),
                        rooted: true,
                    }),
                    rest_at,
                ),
                None => (None, 0),
            }
        } else if live(0) && word.text.starts_with('$') && bare_or_double(word, 0) {
            let (name, end) = variable_name(&word.text).ok_or(Shape::NotLiteral)?;
            if !matches!(word.text[end..].chars().next(), None | Some('/')) {
                return Err(Shape::NotLiteral);
            }
            match self.vars.get(name) {
                Some(Binding::Known(value)) => (Some(value.clone()), end),
                Some(Binding::Unknown(shape)) => return Err(*shape),
                None => return Err(Shape::Unassigned),
            }
        } else {
            (None, 0)
        };
        let mut text = prefix.as_ref().map_or_else(String::new, |p| p.text.clone());
        for (char_at, (at, c)) in word.text.char_indices().enumerate() {
            if at < rest_at {
                continue;
            }
            if live(char_at) && c == '`' {
                return Err(Shape::CommandSubst);
            }
            if live(char_at) && c == '$' {
                let substitution = word.text[at..].starts_with("$(");
                return Err(if substitution {
                    Shape::CommandSubst
                } else {
                    Shape::NotLiteral
                });
            }
            let globbing = matches!(c, '*' | '?' | '[' | '{');
            if bare(char_at) && (globbing || (char_at == 0 && matches!(c, '~' | '='))) {
                return Err(Shape::NotLiteral);
            }
            text.push(c);
        }
        let rooted = prefix.as_ref().is_some_and(|p| p.rooted);
        Ok(Expanded { text, rooted })
    }

    /// The directories of a boundary's `-C` words, or, with none, of the
    /// directory the walk reached; `unresolved` is the shape that kept it
    /// from reading them, in which case the dirs are the payload `cwd`'s.
    fn directories(
        &self,
        payload_cwd: &Path,
        home: Option<&Path>,
        words: &[&Word],
    ) -> (Vec<PathBuf>, Option<Shape>) {
        match self.resolve_directories(payload_cwd, home, words) {
            Ok(dirs) => (dirs, None),
            Err(shape) => (chain(payload_cwd, home, words), Some(shape)),
        }
    }

    /// With no `-C`, `here` is checked unless it is where the call began
    /// and also the payload cwd, which the check covers already.
    fn resolve_directories(
        &self,
        payload_cwd: &Path,
        home: Option<&Path>,
        words: &[&Word],
    ) -> Result<Vec<PathBuf>, Shape> {
        if words.is_empty() {
            let here = self.here.clone()?;
            return Ok(if here == self.start && here == payload_cwd {
                Vec::new()
            } else {
                vec![here]
            });
        }
        let mut base = self.here.clone();
        for word in words {
            let expanded = self.expand(word, home)?;
            base = Ok(expanded.path(base.as_deref().map_err(|shape| *shape))?);
        }
        Ok(vec![base?])
    }
}

fn bare_or_double(word: &Word, at: usize) -> bool {
    matches!(word.quoting.get(at), Some(Quoting::Bare | Quoting::Double))
}

/// The name in a leading `$NAME` or `${NAME}`, and where it ends.
fn variable_name(text: &str) -> Option<(&str, usize)> {
    if let Some(braced) = text.strip_prefix("${") {
        let close = braced.find('}')?;
        let name = &braced[..close];
        return is_identifier(name).then_some((name, close + 3));
    }
    let after = &text[1..];
    let length = after
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(after.len());
    let name = &after[..length];
    is_identifier(name).then_some((name, length + 1))
}

fn classify(words: &[Word], walk: &Walk, cwd: &Path, home: Option<&Path>) -> Option<Reached> {
    let words = command_words(words);
    let texts: Vec<String> = words.iter().map(|word| word.text.clone()).collect();
    if texts.iter().any(|word| base_name(word) == "sweep.py")
        && texts.iter().any(|word| word == "--apply")
    {
        return Some(("sweep.py --apply", Vec::new(), None));
    }
    let call = git_call(&texts)?;
    let directories: Vec<&Word> = call.directories.iter().map(|&at| &words[at]).collect();
    let boundary = |what: &'static str| {
        let (dirs, unresolved) = walk.directories(cwd, home, &directories);
        (what, dirs, unresolved)
    };
    match call.subcommand {
        "worktree" => {
            let verb = call.rest.iter().find(|word| !word.starts_with('-'))?;
            (verb == "remove").then(|| boundary("worktree remove"))
        }
        "commit" => {
            let subject = commit_subject(call.rest)?;
            (subject.starts_with("docs(plan): ") && !is_closing_plan_commit(&subject))
                .then(|| boundary("plan commit"))
        }
        _ => None,
    }
}

/// The first line of the first `-m` message of a `git commit`.
fn commit_subject(rest: &[String]) -> Option<String> {
    let message = rest.iter().enumerate().find_map(|(at, word)| {
        if word == "-m" || word == "--message" {
            return rest.get(at + 1).cloned();
        }
        if let Some(attached) = word.strip_prefix("--message=") {
            return Some(attached.to_string());
        }
        let bundle = word.strip_prefix('-').filter(|b| !b.starts_with('-'))?;
        let position = bundle.find('m')?;
        if !bundle[..position].chars().all(char::is_alphabetic) {
            return None;
        }
        let attached = &bundle[position + 1..];
        if attached.is_empty() {
            rest.get(at + 1).cloned()
        } else {
            Some(attached.to_string())
        }
    })?;
    let body = match message.strip_prefix("$(cat <<") {
        Some(heredoc) => heredoc.split_once('\n')?.1.trim_start().to_string(),
        None => message,
    };
    body.lines().next().map(str::to_string)
}

/// A `docs(plan): …, done` or `…, phase N` commit closes a plan.
fn is_closing_plan_commit(subject: &str) -> bool {
    let subject = subject.trim_end();
    if subject.ends_with(", done") {
        return true;
    }
    subject
        .rsplit_once(", phase ")
        .is_some_and(|(_, number)| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
}

/// What the transcript shows of the relais runs this session started.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelaisFlight {
    /// Starts not yet followed by a finished or exited message.
    pub pending: usize,
    /// `(run id, state)` of runs that finished waiting on this session's
    /// answer and no `mcp__relais__decide` has answered.
    pub waiting: Vec<(String, String)>,
}

impl RelaisFlight {
    pub fn in_flight(&self) -> bool {
        self.pending > 0 || !self.waiting.is_empty()
    }

    /// What one relais plugin message changes: a successor launched adds a
    /// run, a run finished or relais exited takes one off.
    fn apply(&mut self, message: &str) {
        use relais_messages::{
            EXITED, FINISHED, RUN_ID, SUCCESSOR, WAITING_STATES, WITHOUT_OUTCOME,
        };
        if message.contains(SUCCESSOR) {
            self.pending += 1;
        }
        if let Some(id) = word_after(message, RUN_ID).filter(|id| id.starts_with("run-")) {
            if let Some(state) = word_after(message, FINISHED) {
                self.pending = self.pending.saturating_sub(1);
                self.waiting.retain(|(waiting, _)| waiting != id);
                if WAITING_STATES.contains(&state) {
                    self.waiting.push((id.to_string(), state.to_string()));
                }
            }
        } else if message.contains(EXITED) && message.contains(WITHOUT_OUTCOME) {
            self.pending = self.pending.saturating_sub(1);
        }
    }
}

/// The relais wording this rule reads, from relais's
/// `claude-plugin/hooks/runs.ts`. It is relais's text, not a published
/// format: when relais rewords a phrase, a start or an end goes unseen. Each
/// phrase is read so that a miss errs towards a run in flight (an end not
/// recognised keeps the run pending, and the rule silent), except a start
/// not recognised, which the plan's Decision log names as the one contract
/// to recheck at each relais release.
pub(crate) mod relais_messages {
    /// The engine's frame around a message the relais plugin sent.
    pub const PLUGIN_MESSAGE_PREFIX: &str = "The relais plugin sent a message:\n";
    /// The start of an `mcp__relais__run` result that started a run.
    pub const STARTED_PREFIX: &str = "Started relais run";
    /// A plugin message launching a successor run.
    pub const SUCCESSOR: &str = "launches successor";
    /// Followed by the run id, in `relais run <id> finished: <state>`.
    pub const RUN_ID: &str = "relais run ";
    /// Followed by the run's final state.
    pub const FINISHED: &str = "finished: ";
    /// `relais exited [with code N] without an outcome`.
    pub const EXITED: &str = "relais exited";
    pub const WITHOUT_OUTCOME: &str = "without an outcome";
    /// States a finished run waits on this session in.
    pub const WAITING_STATES: [&str; 3] = ["needs_review", "needs_decision", "interrupted"];
}

fn result_text(block: &Value) -> String {
    match &block["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The word after `needle` (`run-65abc-1f2e`, `needs_decision`).
pub(crate) fn word_after<'a>(text: &'a str, needle: &str) -> Option<&'a str> {
    let rest = &text[text.find(needle)? + needle.len()..];
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(rest.len());
    Some(&rest[..end]).filter(|word| !word.is_empty())
}

/// Walks the entries in order and counts the relais runs still pending: a
/// `mcp__relais__run` result that starts `Started relais run` adds one, a
/// plugin message that `launches successor` adds one, and a `finished` or
/// `exited ... without an outcome` plugin message takes one off. A run that
/// finished in `needs_review`, `needs_decision` or `interrupted` waits until
/// a later `mcp__relais__decide` names it.
pub fn relais_flight(entries: &[Value]) -> RelaisFlight {
    let mut tool_names = std::collections::HashMap::new();
    let mut flight = RelaisFlight::default();
    for entry in entries {
        match entry_type(entry) {
            "assistant" => {
                for block in blocks(entry) {
                    if block["type"].as_str() != Some("tool_use") {
                        continue;
                    }
                    let name = block["name"].as_str().unwrap_or("");
                    if let Some(id) = block["id"].as_str() {
                        tool_names.insert(id.to_string(), name.to_string());
                    }
                    if name == "mcp__relais__decide" {
                        if let Some(run) = block["input"]["run"].as_str() {
                            flight.waiting.retain(|(waiting, _)| waiting != run);
                        }
                    }
                }
            }
            "user" => {
                if let Some(message) = entry
                    .pointer("/message/content")
                    .and_then(Value::as_str)
                    .and_then(|text| text.strip_prefix(relais_messages::PLUGIN_MESSAGE_PREFIX))
                {
                    flight.apply(message);
                    continue;
                }
                for block in blocks(entry) {
                    let started = block["type"].as_str() == Some("tool_result")
                        && block["tool_use_id"]
                            .as_str()
                            .and_then(|id| tool_names.get(id))
                            .is_some_and(|name| name == "mcp__relais__run")
                        && result_text(block)
                            .trim_start()
                            .starts_with(relais_messages::STARTED_PREFIX);
                    if started {
                        flight.pending += 1;
                    }
                }
            }
            _ => {}
        }
    }
    flight
}

/// Whose repository is not clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotCleanOf {
    /// The payload `cwd`'s, which a journal row's `repo` already names.
    PayloadCwd,
    /// One a boundary's directory named.
    NamedDirectory,
}

/// How the repositories a boundary touched stand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repositories {
    /// Every checked repository is clean, with its HEAD (empty when unborn).
    Clean { heads: Vec<(PathBuf, String)> },
    NotClean {
        directory: PathBuf,
        reason: String,
        /// Whose repository it is.
        of: NotCleanOf,
    },
    /// The shared deadline passed.
    TimedOut,
}

fn run(directory: &Path, args: &[&str], deadline: Instant) -> Result<Vec<u8>, BoundedFailure> {
    bytes_bounded_detail(directory, args, None, &[], deadline)
}

fn run_text(directory: &Path, args: &[&str], deadline: Instant) -> Result<String, BoundedFailure> {
    run(directory, args, deadline).map(|b| String::from_utf8_lossy(&b).trim().to_string())
}

/// How one repository stands.
enum RepoCheck {
    /// Clean, with its HEAD (empty when unborn).
    Clean(String),
    NotClean(String),
    TimedOut,
}

/// The payload `cwd` when inside a work tree, and each existing `directory`:
/// a missing one, or one that is no work tree, counts as clean. A repository
/// is not clean when its status lists anything, or its branch has commits off
/// its upstream (its base when it has none) that touch a path outside
/// `docs/plans/`. Every git child shares `deadline`.
pub fn check_repositories(
    cwd: Option<&Path>,
    directories: &[PathBuf],
    deadline: Instant,
) -> Repositories {
    let mut heads: Vec<(PathBuf, String)> = Vec::new();
    let cwd_count = usize::from(cwd.is_some());
    for (at, directory) in cwd
        .into_iter()
        .chain(directories.iter().map(PathBuf::as_path))
        .enumerate()
    {
        if !directory.is_dir() {
            continue;
        }
        let top = match run_text(directory, &["rev-parse", "--show-toplevel"], deadline) {
            Ok(top) => PathBuf::from(top),
            Err(BoundedFailure::TimedOut) => return Repositories::TimedOut,
            Err(BoundedFailure::Exited { .. }) => continue,
        };
        if heads.iter().any(|(seen, _)| *seen == top) {
            continue;
        }
        match check_one(&top, deadline) {
            RepoCheck::Clean(head) => heads.push((top, head)),
            RepoCheck::NotClean(reason) => {
                return Repositories::NotClean {
                    directory: top,
                    reason,
                    of: not_clean_of(at, cwd_count),
                }
            }
            RepoCheck::TimedOut => return Repositories::TimedOut,
        }
    }
    Repositories::Clean { heads }
}

fn not_clean_of(at: usize, cwd_count: usize) -> NotCleanOf {
    if at < cwd_count {
        NotCleanOf::PayloadCwd
    } else {
        NotCleanOf::NamedDirectory
    }
}

/// A git refusal is a reason the repository cannot be called clean, named
/// after the command git refused.
fn refused(what: &str, failure: BoundedFailure) -> RepoCheck {
    match failure {
        BoundedFailure::TimedOut => RepoCheck::TimedOut,
        BoundedFailure::Exited { .. } => RepoCheck::NotClean(format!("git {what} failed")),
    }
}

fn check_one(top: &Path, deadline: Instant) -> RepoCheck {
    let head = match run_text(top, &["rev-parse", "HEAD"], deadline) {
        Ok(head) => head,
        Err(BoundedFailure::TimedOut) => return RepoCheck::TimedOut,
        Err(BoundedFailure::Exited { .. }) => String::new(), // an unborn branch has no HEAD
    };
    let status = match run(top, &["status", "--porcelain", "-z"], deadline) {
        Ok(status) => status,
        Err(failure) => return refused("status", failure),
    };
    if !status.is_empty() {
        return RepoCheck::NotClean("uncommitted changes".to_string());
    }
    let names = match unpushed_names(top, "@{upstream}..HEAD", deadline) {
        Ok(names) => names,
        Err(BoundedFailure::TimedOut) => return RepoCheck::TimedOut,
        Err(BoundedFailure::Exited { .. }) => {
            let Some((base, _)) = greenbelay_guard::git::default_ref(top, deadline) else {
                return if Instant::now() >= deadline {
                    RepoCheck::TimedOut
                } else {
                    RepoCheck::NotClean("no upstream or base to compare with".to_string())
                };
            };
            match unpushed_names(top, &format!("{base}..HEAD"), deadline) {
                Ok(names) => names,
                Err(failure) => return refused("log", failure),
            }
        }
    };
    if names.iter().any(|name| !name.starts_with("docs/plans/")) {
        return RepoCheck::NotClean("unpushed commits outside docs/plans/".to_string());
    }
    RepoCheck::Clean(head)
}

/// The paths the commits of `range` touch, read NUL-separated: git quotes a
/// non-ASCII name unless told otherwise.
fn unpushed_names(
    top: &Path,
    range: &str,
    deadline: Instant,
) -> Result<Vec<String>, BoundedFailure> {
    let bytes = run(
        top,
        &["log", "--name-only", "-z", "--format=", range],
        deadline,
    )?;
    Ok(bytes
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect())
}

/// The key of one boundary: its command and the HEAD of each checked
/// repository. The same key again means the same boundary was already advised.
pub fn boundary_key(command: &str, heads: &[(PathBuf, String)]) -> String {
    let mut key = command.to_string();
    for (directory, head) in heads {
        key.push('\n');
        key.push_str(&directory.to_string_lossy());
        key.push(' ');
        key.push_str(head);
    }
    key
}

/// The one deadline every git child of a Stop shares.
pub const DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

fn state_dir() -> Option<PathBuf> {
    Some(journal::dir()?.join("clear-at-boundary"))
}

fn state_file(session: &str) -> Option<PathBuf> {
    if !greenbelay_guard::session_state::usable_id(session) {
        return None;
    }
    Some(state_dir()?.join(session))
}

/// A boundary reached while something was in flight, kept for a later Stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The `uuid` of the transcript entry the boundary command was found in.
    pub uuid: String,
    pub command: String,
    pub what: String,
    pub dirs: Vec<PathBuf>,
    /// A boundary the turn reached whose directory the walk could not read.
    pub unresolved: bool,
}

/// What a session's state file holds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct State {
    /// The opening prompt's uuid of the turn advised in, empty when the rule
    /// only observed.
    pub(crate) turn: String,
    /// The key of the last boundary judged.
    pub(crate) key: String,
    /// The advices given in the session; a file in the old format reads 0.
    pub(crate) advices: u32,
    pub(crate) pending: Option<Pending>,
}

pub(crate) fn parse_pending(pending_json: &Value) -> Option<Pending> {
    let text = |name: &str| pending_json[name].as_str().map(str::to_string);
    Some(Pending {
        uuid: text("uuid")?,
        command: text("command")?,
        what: text("what")?,
        dirs: pending_json["dirs"]
            .as_array()?
            .iter()
            .filter_map(|dir| dir.as_str().map(PathBuf::from))
            .collect(),
        unresolved: pending_json["unresolved"].as_bool().unwrap_or(false),
    })
}

/// The file is JSON `{turn, advices, key, pending}`; the previous text
/// format, `<turn> <advices>\n<key>` or `<turn>\n<key>`, reads with no
/// pending boundary.
fn parse_state(text: &str) -> Option<State> {
    if text.starts_with('{') {
        let state_json: Value = serde_json::from_str(text).ok()?;
        return Some(State {
            turn: state_json["turn"].as_str()?.to_string(),
            key: state_json["key"].as_str()?.to_string(),
            advices: state_json["advices"]
                .as_u64()
                .and_then(|count| u32::try_from(count).ok())
                .unwrap_or(0),
            pending: parse_pending(&state_json["pending"]),
        });
    }
    let (head, key) = text.split_once('\n')?;
    let (turn, advices) = match head.split_once(' ') {
        Some((turn, count)) => (turn, count.parse().unwrap_or(0)),
        None => (head, 0),
    };
    Some(State {
        turn: turn.to_string(),
        key: key.to_string(),
        advices,
        pending: None,
    })
}

fn read_state(session: &str) -> Option<State> {
    parse_state(&std::fs::read_to_string(state_file(session)?).ok()?)
}

pub(crate) fn render_state(state: &State) -> String {
    let pending = state.pending.as_ref().map_or(Value::Null, |pending| {
        serde_json::json!({
            "uuid": pending.uuid,
            "command": pending.command,
            "what": pending.what,
            "dirs": pending.dirs.iter().map(|dir| dir.to_string_lossy()).collect::<Vec<_>>(),
            "unresolved": pending.unresolved,
        })
    });
    serde_json::json!({
        "turn": state.turn,
        "advices": state.advices,
        "key": state.key,
        "pending": pending,
    })
    .to_string()
}

fn write_state(session: &str, state: &State) -> std::io::Result<()> {
    let unusable = || std::io::Error::other("no usable state file for this session");
    let file = state_file(session).ok_or_else(unusable)?;
    let dir = file.parent().ok_or_else(unusable)?;
    std::fs::create_dir_all(dir)?;
    journal::private(dir, 0o700);
    write_atomic(&file, &render_state(state))
}

/// State files older than a week are nobody's session any more.
pub fn sweep() {
    let Some(dir) = state_dir() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let week = std::time::Duration::from_secs(7 * 24 * 3600);
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age > week);
        if old {
            let _ = std::fs::remove_file(entry.path()); // a file already gone is swept
        }
    }
}

/// The rule already spoke in the turn this Stop ends: its state file holds
/// the opening person prompt's uuid, and the transcript still shows the same
/// prompt as the last one. `plan-phases-open` stays silent then, so the Stop
/// after the handoff Continue ends the turn.
pub fn spoke_this_turn(stop: &Stop) -> bool {
    let Some(State { turn, .. }) = read_state(&stop.session) else {
        return false;
    };
    if turn.is_empty() {
        return false;
    }
    let Some(transcript) = stop.transcript.as_deref() else {
        return false;
    };
    let Ok(tail) = read_tail_until(transcript, TAIL_FIRST_CHUNK, TAIL_CAP, |entries| {
        turn_window(entries).is_some()
    }) else {
        return false;
    };
    turn_window(&tail.entries).is_some_and(|window| window.prompt_uuid == turn)
}

/// A `source=clear` session start: the journal line the plan's Outcome (a)
/// counts.
pub fn on_session_start_clear(session: &str, cwd: &Path) {
    let repo = cwd.file_name().map_or_else(
        || "-".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    journal::record(&journal::Entry {
        rule: RULE.id,
        stance: "-",
        outcome: "watched",
        session,
        repo: &repo,
        mode: "-",
        excerpt: "session-start clear",
    });
}

fn has_waiting_line(message: &str) -> bool {
    message
        .lines()
        .any(|line| line.trim().starts_with("WAITING:"))
}

/// What a Stop's payload shows, as far as the judgement reads it.
#[derive(Debug, Clone, Default)]
pub(crate) struct StopView {
    pub cwd: Option<PathBuf>,
    /// What a boundary's `~` or `$HOME` resolves against: `$HOME` live, the
    /// symbolic `~` in a replay (fixtures write home as `~`).
    pub home: Option<PathBuf>,
    pub permission_mode: Option<String>,
    pub background_busy: bool,
    pub stop_hook_active: bool,
    pub ends_with_question: bool,
    pub waiting_line: bool,
    pub marker: bool,
}

impl StopView {
    fn of(stop: &Stop) -> Self {
        let message = stop.last_message.as_deref();
        Self {
            cwd: stop.cwd.clone(),
            home: home_from(std::env::var_os("HOME")),
            permission_mode: stop.permission_mode.clone(),
            background_busy: stop.background_busy,
            stop_hook_active: stop.stop_hook_active,
            ends_with_question: message.is_some_and(|text| text.trim_end().ends_with('?')),
            waiting_line: message.is_some_and(has_waiting_line),
            marker: message.is_some_and(|text| text.contains(MARKER)),
        }
    }

    fn says_silence(&self) -> bool {
        self.stop_hook_active
            || self.permission_mode.as_deref() == Some("plan")
            || self.ends_with_question
            || self.waiting_line
            || self.marker
    }
}

/// Silence without a journal line: what the payload alone shows.
fn payload_says_silence(stop: &Stop) -> bool {
    !greenbelay_guard::session_state::usable_id(&stop.session) || StopView::of(stop).says_silence()
}

/// What the Stop shows about a boundary past [`SIZE`] tokens. `fact` gives
/// the plan's state (`plan_phases::open_phase`), and is called only once the
/// context is past the size, so a smaller Stop spawns no git for this rule.
/// `Working` is in flight, so the rule is silent. Every failure is silence.
/// At `deny` the answer is a Continue, at `advise` a note to the person
/// (except at `PlanOnly`, where `plan-phases-open` carries the plan on), at
/// `observe` silence.
pub fn on_stop(
    stop: &Stop,
    fact: impl FnOnce() -> PhaseFact,
    deadline: Instant,
    stance: impl FnOnce() -> Stance,
) -> Decision {
    if payload_says_silence(stop) {
        return Decision::Silent;
    }
    let Some(transcript) = stop.transcript.as_deref() else {
        return Decision::Silent;
    };
    let prior = read_state(&stop.session).unwrap_or_default();
    let pending_uuid = prior.pending.as_ref().map(|pending| pending.uuid.clone());
    let Ok(tail) = read_tail_until(transcript, TAIL_FIRST_CHUNK, TAIL_CAP, |entries| {
        window_is_enough(entries, pending_uuid.as_deref())
    }) else {
        return Decision::Silent;
    };
    judge_boundary(stop, &tail, prior, fact, deadline, stance)
}

/// Whether a Stop's tail holds what the judgement reads: the context size, the
/// turn window and, when a boundary is pending, the entry it was found in.
/// The one stopping predicate of [`on_stop`] and of the exporter's replay of
/// its window.
pub(crate) fn window_is_enough(entries: &[Value], pending_uuid: Option<&str>) -> bool {
    context_size(entries).is_some()
        && turn_window(entries).is_some()
        && pending_uuid.is_none_or(|uuid| position_of(entries, uuid).is_some())
}

/// The boundary a Stop judges: one this turn reached, or one kept pending
/// from an earlier Stop.
enum Candidate {
    Own(Pending),
    Resumed(Pending),
}

impl Candidate {
    fn boundary(&self) -> &Pending {
        match self {
            Candidate::Own(boundary) | Candidate::Resumed(boundary) => boundary,
        }
    }
}

pub(crate) fn position_of(entries: &[Value], uuid: &str) -> Option<usize> {
    if uuid.is_empty() {
        return None;
    }
    entries
        .iter()
        .position(|entry| entry["uuid"].as_str() == Some(uuid))
}

/// Whether the entries show work started: an Edit, Write or NotebookEdit, a
/// `git worktree add`, or an `mcp__relais__run`.
pub(crate) fn work_in(entries: &[Value]) -> bool {
    entries
        .iter()
        .filter(|entry| entry_type(entry) == "assistant" && !is_sidechain(entry))
        .flat_map(blocks)
        .filter(|block| block["type"].as_str() == Some("tool_use"))
        .any(|block| match block["name"].as_str().unwrap_or("") {
            "Edit" | "Write" | "NotebookEdit" | "mcp__relais__run" => true,
            "Bash" => block
                .pointer("/input/command")
                .and_then(Value::as_str)
                .is_some_and(|command| segments(command).iter().any(|words| adds_worktree(words))),
            _ => false,
        })
}

fn adds_worktree(words: &[String]) -> bool {
    git_call(words).is_some_and(|call| {
        call.subcommand == "worktree"
            && call
                .rest
                .iter()
                .find(|word| !word.starts_with('-'))
                .map(String::as_str)
                == Some("add")
    })
}

/// The journal's excerpt limit; `journal::redact_and_trim` cuts past it.
const EXCERPT_LIMIT: usize = 200;

/// Whether the tail read hit its cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TailCut {
    Whole,
    Capped,
}

impl TailCut {
    fn of(tail: &Tail) -> Self {
        if tail.capped {
            TailCut::Capped
        } else {
            TailCut::Whole
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            TailCut::Whole => "",
            TailCut::Capped => " tail-capped",
        }
    }
}

/// The journal excerpt of a record: `what`, then ` tail-capped` when the tail
/// hit its cap, then ` uuid=<id>` of the tail's last entry. `what` is cut to
/// leave room for the suffix, so `redact_and_trim` never drops the uuid.
fn excerpt(what: &str, cut: TailCut, last_uuid: Option<&str>) -> String {
    let suffix = format!(
        "{}{}",
        cut.suffix(),
        last_uuid.map_or_else(String::new, |uuid| format!(" uuid={uuid}"))
    );
    let room = EXCERPT_LIMIT.saturating_sub(suffix.chars().count());
    let what: String = what.chars().take(room).collect();
    format!("{what}{suffix}")
}

/// What [`decide`] settled: the answer and the journal records, in order, as `(outcome, what)`.
pub(crate) struct Verdict {
    pub decision: Decision,
    pub records: Vec<(String, String)>,
    /// The stance the judgement ran at, `None` when it ended before asking.
    pub stance: Option<Stance>,
}

/// The I/O around [`decide`]: reads the state and the live git facts, writes
/// the state file, journals the records.
fn judge_boundary(
    stop: &Stop,
    tail: &Tail,
    prior: State,
    fact: impl FnOnce() -> PhaseFact,
    deadline: Instant,
    stance: impl FnOnce() -> Stance,
) -> Decision {
    let view = StopView::of(stop);
    let verdict = decide(
        &view,
        &tail.entries,
        prior,
        fact,
        stance,
        |dirs| check_repositories(view.cwd.as_deref(), dirs, deadline),
        |state| write_state(&stop.session, state).is_ok(),
    );
    if let Some(stance) = verdict.stance {
        let repo = view.cwd.as_deref().and_then(Path::file_name).map_or_else(
            || "-".to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let mode = view.permission_mode.as_deref().unwrap_or("-");
        let last_uuid = tail.entries.last().and_then(|entry| entry["uuid"].as_str());
        for (outcome, what) in &verdict.records {
            journal::record(&journal::Entry {
                rule: RULE.id,
                stance: stance.as_str(),
                outcome,
                session: &stop.session,
                repo: &repo,
                mode,
                excerpt: &excerpt(what, TailCut::of(tail), last_uuid),
            });
        }
    }
    verdict.decision
}

/// The records and the state write of one judgement.
struct Ledger<P: FnMut(&State) -> bool> {
    records: Vec<(String, String)>,
    persist: P,
}

impl<P: FnMut(&State) -> bool> Ledger<P> {
    fn record(&mut self, outcome: &str, what: &str) {
        self.records.push((outcome.to_string(), what.to_string()));
    }

    fn watched(&mut self, what: &str) -> Option<Decision> {
        self.record("watched", what);
        None
    }

    /// A state file that cannot be written loses the boundary key and the
    /// count: say so, and let the caller stay silent rather than advise
    /// without them.
    fn save(&mut self, state: &State) -> bool {
        let written = (self.persist)(state);
        if !written {
            self.record("watched", "state-unwritable");
        }
        written
    }

    /// A boundary that cannot be advised yet: an own boundary is saved as
    /// pending for a later Stop, and the Stop stays silent.
    fn hold(&mut self, candidate: &Candidate, mut state: State, reason: &str) -> Option<Decision> {
        if let Candidate::Own(boundary) = candidate {
            state.pending = Some(boundary.clone());
            if !self.save(&state) {
                return None;
            }
        }
        self.watched(&format!("pending {reason}"))
    }

    /// `None` is silence, journalled or not as each check decides.
    fn judge(
        &mut self,
        view: &StopView,
        entries: &[Value],
        prior: State,
        (fact, stance, size): (PhaseFact, Stance, u64),
        repos: impl FnOnce(&[PathBuf]) -> Repositories,
    ) -> Option<Decision> {
        let cwd = view.cwd.as_deref();
        let mut state = prior;
        if fact == PhaseFact::TimedOut {
            return self.watched("timeout");
        }
        let Some(window) = turn_window(entries) else {
            return self.watched("turn-too-long");
        };
        let found = boundaries(&window, cwd.unwrap_or(Path::new("")), view.home.as_deref());
        let own = found.last().filter(|boundary| {
            state
                .pending
                .as_ref()
                .is_none_or(|pending| pending.uuid != boundary.uuid)
        });
        let candidate = match (own, state.pending.clone()) {
            (Some(boundary), _) => Candidate::Own(pending_from(&found, boundary)),
            (None, None) => return self.watched("no boundary"),
            (None, Some(pending)) => {
                let Some(at) = position_of(entries, &pending.uuid) else {
                    state.pending = None;
                    self.save(&state);
                    return self.watched("pending-lost");
                };
                let after = &entries[at + 1..];
                if after.iter().any(is_person_prompt) {
                    state.pending = None;
                    self.save(&state);
                    return self.watched("no boundary");
                }
                if work_in(after) {
                    return self.watched("pending-work");
                }
                Candidate::Resumed(pending)
            }
        };
        // The row names the Stop's own, last boundary, and goes again only
        // when the boundary is a new one: see the key check below.
        let mut unresolved_row = None;
        if let Some(Boundary {
            unresolved: Some(shape),
            what,
            ..
        }) = own
        {
            unresolved_row = Some(self.records.len());
            self.record("watched", &format!("unresolved {} {what}", shape.as_str()));
        }
        if view.background_busy {
            return self.hold(&candidate, state, "background");
        }
        let flight = relais_flight(entries);
        if let Some((id, relais_state)) = flight.waiting.first() {
            return self.hold(&candidate, state, &format!("relais {id} {relais_state}"));
        }
        if flight.pending > 0 {
            return self.hold(
                &candidate,
                state,
                &format!("relais pending {}", flight.pending),
            );
        }
        let boundary = candidate.boundary();
        let heads = match repos(&boundary.dirs) {
            Repositories::Clean { heads } => heads,
            Repositories::NotClean {
                reason,
                directory,
                of,
            } => {
                let named = (of == NotCleanOf::NamedDirectory)
                    .then(|| directory.file_name())
                    .flatten();
                let reason = match named {
                    Some(name) => format!("{reason} dir={}", name.to_string_lossy()),
                    None => reason,
                };
                return self.hold(&candidate, state, &reason);
            }
            Repositories::TimedOut => return self.watched("timeout"),
        };
        let key = boundary_key(&boundary.command, &heads);
        let advices = state.advices;
        if state.key == key {
            if let Some(at) = unresolved_row {
                self.records.remove(at);
            }
            if state.pending.is_some() {
                state.pending = None;
                self.save(&state);
            }
            return None;
        }
        if advices >= MAX_ADVICES_PER_SESSION {
            // The key, saved alone, sends a repeat Stop on this boundary
            // through the key check above: no second `cap` or `unresolved`
            // row. A failed save journals `state-unwritable`, and the next
            // Stop writes the rows again, as before.
            state.key = key;
            let _saved = self.save(&state);
            return self.watched("cap");
        }
        // At `advise` a note cannot carry the plan on, and a lone plan commit's
        // next step is the plan's: `plan-phases-open` speaks instead.
        let carried_by_plan = stance == Stance::Advise && fact == PhaseFact::PlanOnly;
        let counted = stance != Stance::Observe && !carried_by_plan;
        // Only a Continue asks for a handoff Stop, which `spoke_this_turn` lets end.
        let turn = if stance == Stance::Deny {
            window.prompt_uuid.as_str()
        } else {
            ""
        };
        let state = State {
            turn: turn.to_string(),
            key,
            advices: advices + u32::from(counted),
            pending: None,
        };
        if !self.save(&state) {
            return None;
        }
        if carried_by_plan {
            return self.watched(&format!(
                "{} (advise, plan-phases-open carries the plan)",
                boundary.what
            ));
        }
        let fact = format!(
            "this turn reached a boundary ({}) with nothing in flight, and the context is at {}k tokens, past {}k.",
            boundary.what,
            size / 1000,
            SIZE / 1000
        );
        match stance {
            Stance::Observe => self.watched(&format!("{} (observe)", boundary.what)),
            Stance::Advise => {
                self.record("advised", &boundary.what);
                // The agent hands off unasked (worktree-task `## Handoff`), so
                // the note names that step; the line it quotes still works with
                // no handoff skill installed.
                let remedy = format!(
                    "The agent hands off by itself at a boundary, as the worktree-task skill's Handoff section says. Without that skill, the handoff is complete when the plan or ticket states the next action. Either way, the turn ends with the line \"{MARKER}; next: `<the exact first prompt>`\". This turn ended without that line. To clear and carry on, reply: hand off now. The clear-relay plugin then clears and submits that prompt. Or /clear yourself."
                );
                Some(Decision::UserNote(decision::phrase(
                    RULE.id, &fact, &remedy,
                )))
            }
            Stance::Deny => {
                self.record("denied", &boundary.what);
                let remedy = format!(
                    "Hand off now, as the worktree-task skill's Handoff section says. Without that skill, the handoff is complete when the plan or ticket states the next action and the turn ends with one line: \"{MARKER}; next: `<the exact first prompt>`\"."
                );
                Some(Decision::Continue(decision::phrase(
                    RULE.id, &fact, &remedy,
                )))
            }
        }
    }
}

/// The judgement, free of I/O. `fact` gives the plan's state and is called
/// only once the context is past [`SIZE`]; `repos` checks the repositories a
/// boundary touched; `persist` writes the state and says whether it held.
/// Every failure is silence.
pub(crate) fn decide(
    view: &StopView,
    entries: &[Value],
    prior: State,
    fact: impl FnOnce() -> PhaseFact,
    stance: impl FnOnce() -> Stance,
    repos: impl FnOnce(&[PathBuf]) -> Repositories,
    persist: impl FnMut(&State) -> bool,
) -> Verdict {
    let silent = Verdict {
        decision: Decision::Silent,
        records: Vec::new(),
        stance: None,
    };
    if view.says_silence() {
        return silent;
    }
    let Some(size) = context_size(entries).filter(|size| *size >= SIZE) else {
        return silent;
    };
    let fact = fact();
    if fact == PhaseFact::Working {
        return silent;
    }
    let stance = stance();
    let mut ledger = Ledger {
        records: Vec::new(),
        persist,
    };
    let decision = ledger.judge(view, entries, prior, (fact, stance, size), repos);
    Verdict {
        decision: decision.unwrap_or(Decision::Silent),
        records: ledger.records,
        stance: Some(stance),
    }
}

/// The boundary a turn kept pending: `boundary`, with the directories of
/// every boundary the turn reached.
pub(crate) fn pending_from(found: &[Boundary], boundary: &Boundary) -> Pending {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in found.iter().flat_map(|boundary| boundary.dirs.iter()) {
        if !dirs.contains(dir) {
            dirs.push(dir.clone());
        }
    }
    Pending {
        uuid: boundary.uuid.clone(),
        command: boundary.command.clone(),
        what: boundary.what.clone(),
        dirs,
        unresolved: found.iter().any(|boundary| boundary.unresolved.is_some()),
    }
}

/// A recorded Stop as `explain --format cases` writes it; members the replay
/// does not read (`source`, `early`, `inferred`) are left out.
#[derive(Deserialize)]
struct StopFixture {
    payload: PayloadFixture,
    state: StateFixture,
    facts: FactsFixture,
    entries: Vec<Value>,
    #[serde(default = "countable_by_default")]
    countable: bool,
    excluded: Option<String>,
}

fn countable_by_default() -> bool {
    true
}

#[derive(Deserialize)]
struct PayloadFixture {
    cwd: Option<String>,
    permission_mode: Option<String>,
    background_busy: bool,
    stop_hook_active: bool,
    ends_with_question: bool,
    waiting_line: bool,
    marker: bool,
}

#[derive(Deserialize)]
struct StateFixture {
    #[serde(default)]
    turn: String,
    key: String,
    advices: u32,
    pending: Option<PendingFixture>,
}

#[derive(Deserialize)]
struct PendingFixture {
    uuid: String,
    command: String,
    what: String,
    dirs: Vec<String>,
    #[serde(default)]
    unresolved: bool,
}

#[derive(Deserialize)]
struct FactsFixture {
    phase: PhaseFixture,
    repositories: RepositoriesFixture,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum PhaseFixture {
    #[serde(rename = "none")]
    NoOpenPhase,
    PlanOnly,
    Working,
    TimedOut,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RepositoriesFixture {
    Clean,
    TimedOut,
    NotClean(String),
    /// The fact per directory, keyed by the directory the rule resolves in
    /// replay (home as `~`), not by its git toplevel. A value is `clean`,
    /// `timed_out` or a not-clean reason; a directory the map does not name
    /// is clean.
    PerDirectory(BTreeMap<String, String>),
}

/// The git facts a fixture froze.
#[derive(Debug, Clone)]
pub(crate) enum FrozenRepositories {
    /// One answer whatever the directories.
    All(Repositories),
    PerDirectory(BTreeMap<PathBuf, DirectoryFact>),
}

/// The reasons [`check_one`] gives besides a git refusal; a fixture reason
/// outside them (a typo) is refused rather than frozen as dirty.
const NOT_CLEAN_REASONS: [&str; 3] = [
    "uncommitted changes",
    "no upstream or base to compare with",
    "unpushed commits outside docs/plans/",
];

/// A reason [`refused`] gives: `git <command> failed`.
fn is_git_refusal(reason: &str) -> bool {
    reason
        .strip_prefix("git ")
        .and_then(|rest| rest.strip_suffix(" failed"))
        .is_some_and(|command| !command.is_empty() && !command.contains(' '))
}

/// One directory's frozen git fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DirectoryFact {
    Clean,
    TimedOut,
    NotClean(String),
}

impl DirectoryFact {
    /// A `per_directory` member: its key must be a path the rule can resolve
    /// to (rooted, or under the symbolic home), and its value `clean`,
    /// `timed_out` or a reason the live check gives.
    fn parse(directory: String, fact: String) -> Result<(PathBuf, Self), FixtureError> {
        let path = PathBuf::from(&directory);
        // `has_root`, not `is_absolute`: a scrubbed fixture path such as
        // `/work` has no drive, so Windows never calls it absolute.
        let rooted = path.has_root() || path.starts_with(SYMBOLIC_HOME);
        if !rooted {
            return Err(FixtureError::WrongType(format!(
                "per_directory key {directory:?} is neither rooted nor under {SYMBOLIC_HOME}"
            )));
        }
        let fact = match fact.as_str() {
            "clean" => Self::Clean,
            "timed_out" => Self::TimedOut,
            reason if NOT_CLEAN_REASONS.contains(&reason) || is_git_refusal(reason) => {
                Self::NotClean(fact)
            }
            _ => {
                return Err(FixtureError::WrongType(format!(
                    "per_directory {directory:?}: {fact:?} is not clean, timed_out or a reason the check gives"
                )))
            }
        };
        Ok((path, fact))
    }
}

impl FrozenRepositories {
    /// The answer for `cwd` and `directories`, walked in the order
    /// [`check_repositories`] walks them: the first not-clean or timed-out
    /// directory decides.
    pub(crate) fn answer(&self, cwd: Option<&Path>, directories: &[PathBuf]) -> Repositories {
        let map = match self {
            Self::All(repositories) => return repositories.clone(),
            Self::PerDirectory(map) => map,
        };
        let cwd_count = usize::from(cwd.is_some());
        for (at, directory) in cwd
            .into_iter()
            .chain(directories.iter().map(PathBuf::as_path))
            .enumerate()
        {
            match map.get(directory) {
                None | Some(DirectoryFact::Clean) => {}
                Some(DirectoryFact::TimedOut) => return Repositories::TimedOut,
                Some(DirectoryFact::NotClean(reason)) => {
                    return Repositories::NotClean {
                        directory: directory.to_path_buf(),
                        reason: reason.clone(),
                        of: not_clean_of(at, cwd_count),
                    }
                }
            }
        }
        Repositories::Clean { heads: Vec::new() }
    }
}

/// What a countable fixture replays with.
struct StopInputs {
    view: StopView,
    entries: Vec<Value>,
    prior: State,
    phase: PhaseFact,
    repositories: FrozenRepositories,
}

impl TryFrom<StopFixture> for StopInputs {
    type Error = FixtureError;

    fn try_from(fixture: StopFixture) -> Result<Self, FixtureError> {
        if !fixture.countable {
            return Err(FixtureError::NotCountable(
                fixture
                    .excluded
                    .unwrap_or_else(|| "no reason given".to_string()),
            ));
        }
        let payload = fixture.payload;
        let view = StopView {
            cwd: payload.cwd.map(PathBuf::from),
            home: Some(PathBuf::from(SYMBOLIC_HOME)),
            permission_mode: payload.permission_mode,
            background_busy: payload.background_busy,
            stop_hook_active: payload.stop_hook_active,
            ends_with_question: payload.ends_with_question,
            waiting_line: payload.waiting_line,
            marker: payload.marker,
        };
        let prior = State {
            turn: fixture.state.turn,
            key: fixture.state.key,
            advices: fixture.state.advices,
            pending: fixture.state.pending.map(|pending| Pending {
                uuid: pending.uuid,
                command: pending.command,
                what: pending.what,
                dirs: pending.dirs.into_iter().map(PathBuf::from).collect(),
                unresolved: pending.unresolved,
            }),
        };
        let phase = match fixture.facts.phase {
            PhaseFixture::NoOpenPhase => PhaseFact::NoOpenPhase,
            PhaseFixture::PlanOnly => PhaseFact::PlanOnly,
            PhaseFixture::Working => PhaseFact::Working,
            PhaseFixture::TimedOut => PhaseFact::TimedOut,
        };
        let repositories = match fixture.facts.repositories {
            RepositoriesFixture::Clean => {
                FrozenRepositories::All(Repositories::Clean { heads: Vec::new() })
            }
            RepositoriesFixture::TimedOut => FrozenRepositories::All(Repositories::TimedOut),
            RepositoriesFixture::NotClean(reason) => {
                FrozenRepositories::All(Repositories::NotClean {
                    directory: PathBuf::new(),
                    reason,
                    of: NotCleanOf::PayloadCwd,
                })
            }
            RepositoriesFixture::PerDirectory(map) => FrozenRepositories::PerDirectory(
                map.into_iter()
                    .map(|(directory, fact)| DirectoryFact::parse(directory, fact))
                    .collect::<Result<_, _>>()?,
            ),
        };
        Ok(StopInputs {
            view,
            entries: fixture.entries,
            prior,
            phase,
            repositories,
        })
    }
}

/// A serde failure, as the fixture error it means. Telling a missing member
/// from a wrong type reads serde_json's message text, which is not a stable
/// API: `a_malformed_fixture_is_an_error` pins the wording, so an upgrade
/// that rewords it turns that test red rather than mislabelling silently.
fn fixture_error(error: &serde_json::Error) -> FixtureError {
    use serde_json::error::Category;
    let message = error.to_string();
    match error.classify() {
        Category::Io | Category::Syntax | Category::Eof => FixtureError::NotJson(message),
        Category::Data => match message
            .strip_prefix("missing field `")
            .and_then(|rest| rest.split_once('`'))
        {
            Some((member, _)) => FixtureError::Missing(member.to_string()),
            None => FixtureError::WrongType(message),
        },
    }
}

/// Replay a fixture (the plan's case form) through [`decide`] at `deny`, with
/// the facts it froze and a state write that holds. `Ok(true)` when the
/// decision is a Continue; a malformed or uncountable fixture is an `Err`.
pub fn judge_case(fixture_json: &str) -> Result<bool, FixtureError> {
    let fixture: StopFixture =
        serde_json::from_str(fixture_json).map_err(|error| fixture_error(&error))?;
    let inputs = StopInputs::try_from(fixture)?;
    Ok(replay_at_deny(
        &inputs.view,
        &inputs.entries,
        inputs.prior,
        inputs.phase,
        |dirs| inputs.repositories.answer(inputs.view.cwd.as_deref(), dirs),
    )
    .fired)
}

/// What a replay at `deny` settled.
pub(crate) struct Replay {
    /// The decision is a Continue.
    pub fired: bool,
    pub records: Vec<(String, String)>,
    /// The last state the judgement persisted.
    pub saved: Option<State>,
}

/// [`decide`] at `deny` with frozen facts and a state write that holds.
pub(crate) fn replay_at_deny(
    view: &StopView,
    entries: &[Value],
    prior: State,
    phase: PhaseFact,
    repositories: impl FnOnce(&[PathBuf]) -> Repositories,
) -> Replay {
    let mut saved = None;
    let verdict = decide(
        view,
        entries,
        prior,
        || phase,
        || Stance::Deny,
        repositories,
        |state| {
            saved = Some(state.clone());
            true
        },
    );
    Replay {
        fired: matches!(verdict.decision, Decision::Continue(_)),
        records: verdict.records,
        saved,
    }
}

/// The replay of a Stop case for `rule_id`; `None` for a rule this module
/// does not judge.
pub fn stop_judge(rule_id: &str, fixture: &str) -> Option<Result<bool, FixtureError>> {
    (rule_id == RULE.id).then(|| judge_case(fixture))
}

#[cfg(test)]
pub(crate) mod testrepo {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A throwaway repository on branch `main`, removed on drop.
    pub(crate) struct TestRepo {
        directory: PathBuf,
    }

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    impl TestRepo {
        pub(crate) fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "amont-clear-boundary-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&directory).expect("create test repository directory");
            let repo = Self { directory };
            repo.git(&["init", "-q", "-b", "main"]);
            repo
        }

        pub(crate) fn path(&self) -> &Path {
            &self.directory
        }

        pub(crate) fn git(&self, args: &[&str]) {
            let output = Command::new("git")
                .arg("-C")
                .arg(&self.directory)
                .args([
                    "-c",
                    "user.name=test",
                    "-c",
                    "user.email=test@example.com",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .expect("run git");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        pub(crate) fn write(&self, relative: &str, content: &str) {
            let path = self.directory.join(relative);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("create parents");
            std::fs::write(path, content).expect("write file");
        }

        pub(crate) fn commit_file(&self, relative: &str, content: &str, message: &str) {
            self.write(relative, content);
            self.git(&["add", "--", relative]);
            self.git(&["commit", "-q", "-m", message]);
        }
    }

    impl Drop for TestRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory); // a leftover temp directory is harmless
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testrepo::TestRepo;
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    fn assistant(usage: Option<(u64, u64, u64)>, sidechain: bool, blocks: Value) -> Value {
        let mut message = json!({"role": "assistant", "content": blocks});
        if let Some((input, creation, read)) = usage {
            message["usage"] = json!({
                "input_tokens": input,
                "cache_creation_input_tokens": creation,
                "cache_read_input_tokens": read,
            });
        }
        json!({"type": "assistant", "isSidechain": sidechain, "message": message})
    }

    fn prompt(uuid: &str) -> Value {
        json!({"type": "user", "uuid": uuid, "message": {"role": "user", "content": "go"}})
    }

    fn bash(id: &str, command: &str) -> Value {
        assistant(
            None,
            false,
            json!([{"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command}}]),
        )
    }

    fn tool_use(id: &str, name: &str) -> Value {
        assistant(
            None,
            false,
            json!([{"type": "tool_use", "id": id, "name": name, "input": {}}]),
        )
    }

    fn tool_result(id: &str, text: &str) -> Value {
        json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": id, "content": [{"type": "text", "text": text}]}
        ]}})
    }

    fn found(commands: &[&str]) -> Vec<Boundary> {
        let mut entries = vec![prompt("p1")];
        for (at, command) in commands.iter().enumerate() {
            entries.push(bash(&format!("b{at}"), command));
        }
        let window = turn_window(&entries).expect("a window");
        boundaries(&window, Path::new("/work"), None)
    }

    /// The `-C` directories of each boundary in `command`, with `cwd` and `home`.
    fn dirs_with_home(command: &str, cwd: &str, home: Option<&str>) -> Vec<PathBuf> {
        let entries = vec![prompt("p1"), bash("b0", command)];
        let window = turn_window(&entries).expect("a window");
        boundaries(&window, Path::new(cwd), home.map(Path::new))
            .into_iter()
            .flat_map(|boundary| boundary.dirs)
            .collect()
    }

    fn write_lines(path: &Path, entries: &[Value]) {
        let text: String = entries.iter().map(|e| format!("{e}\n")).collect();
        std::fs::write(path, text).expect("write transcript");
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(20)
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("cb-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch directory");
        directory
    }

    #[test]
    fn the_tail_grows_by_chunks_and_drops_a_cut_first_line() {
        let directory = scratch("tail");
        let path = directory.join("t.jsonl");
        let filler = "x".repeat(200);
        let mut entries = vec![prompt("old")];
        for at in 0..40 {
            entries.push(bash(&format!("b{at}"), &filler));
        }
        entries.push(assistant(Some((1, 2, 3)), false, json!([])));
        write_lines(&path, &entries);
        let tail = read_tail_until(&path, 512, 1 << 20, |e| context_size(e).is_some())
            .expect("read the tail");
        assert!(!tail.capped);
        assert!(tail.entries.len() < entries.len(), "only a tail was read");
        assert_eq!(context_size(&tail.entries), Some(6));
        let everything = read_tail_until(&path, 512, 1 << 20, |_| false).expect("read all");
        assert_eq!(everything.entries.len(), entries.len());
        assert!(!everything.capped);
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn below_the_size_the_plan_state_is_never_asked_for() {
        let directory = scratch("lazy");
        let path = directory.join("t.jsonl");
        write_lines(
            &path,
            &[
                prompt("p1"),
                bash("b1", "git worktree remove ../x"),
                tool_result("b1", "ok"),
                assistant(Some((10, 0, 149_990)), false, json!([])),
            ],
        );
        let stop = Stop {
            session: "lazy-session".to_string(),
            cwd: Some(directory.clone()),
            permission_mode: None,
            last_message: None,
            background_busy: false,
            stop_hook_active: false,
            transcript: Some(path),
        };
        let asked = std::cell::Cell::new(false);
        let decision = on_stop(
            &stop,
            || {
                asked.set(true);
                PhaseFact::NoOpenPhase
            },
            deadline(),
            || Stance::Deny,
        );
        assert!(matches!(decision, Decision::Silent));
        assert!(!asked.get(), "open_phase was asked for at 150k");
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn the_cap_is_reported_when_the_file_start_is_not_reached() {
        let directory = scratch("cap");
        let path = directory.join("t.jsonl");
        let filler = "y".repeat(300);
        let entries: Vec<Value> = (0..30).map(|at| bash(&format!("b{at}"), &filler)).collect();
        write_lines(&path, &entries);
        let tail = read_tail_until(&path, 256, 1024, |_| false).expect("read");
        assert!(tail.capped);
        assert!(tail.entries.len() < 30);
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn a_sidechain_usage_after_the_main_chain_one_is_ignored() {
        let entries = vec![
            assistant(Some((10, 20, 30)), false, json!([])),
            assistant(Some((1_000_000, 0, 0)), true, json!([])),
        ];
        assert_eq!(context_size(&entries), Some(60));
    }

    #[test]
    fn the_size_sums_the_three_usage_fields_of_the_last_entry_with_usage() {
        let entries = vec![
            assistant(Some((5, 5, 5)), false, json!([])),
            assistant(Some((100_000, 50_000, 25_000)), false, json!([])),
            assistant(None, false, json!([])),
        ];
        assert_eq!(context_size(&entries), Some(175_000));
        assert_eq!(context_size(&[]), None);
    }

    #[test]
    fn no_person_prompt_in_the_tail_is_no_window() {
        let entries = vec![
            bash("b", "ls"),
            tool_result("b", "files"),
            assistant(Some((1, 1, 1)), false, json!([])),
        ];
        assert!(turn_window(&entries).is_none());
    }

    #[test]
    fn the_window_starts_after_the_last_prompt_that_is_no_tool_result() {
        let entries = vec![
            prompt("first"),
            bash("b", "ls"),
            prompt("second"),
            bash("c", "pwd"),
            tool_result("c", "/"),
        ];
        let window = turn_window(&entries).expect("a window");
        assert_eq!(window.prompt_uuid, "second");
        assert_eq!(window.entries.len(), 2);
    }

    #[test]
    fn every_boundary_shape_is_found_with_its_label() {
        let worktree = found(&["git worktree remove ../wt"]);
        assert_eq!(worktree.len(), 1);
        assert_eq!(worktree[0].what, "worktree remove");
        assert_eq!(found(&["git -C repo worktree remove --force x"]).len(), 1);
        assert_eq!(
            found(&["python3 ~/bin/sweep.py --prune --apply"])[0].what,
            "sweep.py --apply"
        );
        assert!(found(&["python3 sweep.py --dry-run"]).is_empty());
        assert!(found(&["python3 other.py --apply"]).is_empty());
        let plan = found(&["git commit -m \"docs(plan): add the plan\""]);
        assert_eq!(plan[0].what, "plan commit");
        let heredoc = found(&["git commit -m \"$(cat <<'EOF'\ndocs(plan): x\n\nbody\nEOF\n)\""]);
        assert_eq!(heredoc.len(), 1);
        let chained = found(&["cd x && git add . && git commit -am 'docs(plan): y'"]);
        assert_eq!(chained.len(), 1);
        assert!(found(&["git commit -m 'feat: x'"]).is_empty());
    }

    #[test]
    fn closing_plan_commits_are_not_boundaries() {
        assert!(found(&["git commit -m 'docs(plan): x, done'"]).is_empty());
        assert!(found(&["git commit -m 'docs(plan): x, phase 2'"]).is_empty());
        assert_eq!(found(&["git commit -m 'docs(plan): phases'"]).len(), 1);
    }

    #[test]
    fn dash_c_directories_resolve_against_the_cwd_and_chain() {
        let relative = found(&["git -C repo worktree remove w"]);
        assert_eq!(relative[0].dirs, vec![PathBuf::from("/work/repo")]);
        let absolute = found(&["git -C /abs worktree remove w"]);
        assert_eq!(absolute[0].dirs, vec![PathBuf::from("/abs")]);
        let chained = found(&["git -C repo -C ../other worktree remove w"]);
        assert_eq!(chained[0].dirs, vec![PathBuf::from("/work/other")]);
        let none = found(&["git worktree remove w"]);
        assert!(none[0].dirs.is_empty());
    }

    const STARTED: &str =
        "Started relais run for: /w/.relais/tasks/abc123.json (in /w/repo). Waiting.";

    fn plugin_message(text: &str) -> Value {
        json!({"type": "user", "uuid": "m", "message": {"role": "user",
            "content": format!("The relais plugin sent a message:\n{text}")}})
    }

    fn decide(run: &str) -> Value {
        assistant(
            None,
            false,
            json!([{"type": "tool_use", "id": "d", "name": "mcp__relais__decide", "input": {"run": run}}]),
        )
    }

    fn flight_after(extra: Vec<Value>) -> RelaisFlight {
        let mut entries = vec![
            prompt("p"),
            tool_use("r1", "mcp__relais__run"),
            tool_result("r1", STARTED),
        ];
        entries.extend(extra);
        relais_flight(&entries)
    }

    #[test]
    fn a_start_with_no_finished_message_is_in_flight() {
        let flight = flight_after(vec![]);
        assert_eq!(flight.pending, 1);
        assert!(flight.in_flight());
    }

    #[test]
    fn a_start_in_an_earlier_turn_is_in_flight() {
        assert!(flight_after(vec![prompt("later turn")]).in_flight());
    }

    #[test]
    fn a_finished_accepted_run_is_not_in_flight() {
        let flight = flight_after(vec![plugin_message(
            "relais run run-65abc-1f2e finished: accepted.",
        )]);
        assert!(!flight.in_flight());
    }

    #[test]
    fn a_run_that_needs_a_decision_is_in_flight_until_decided() {
        let finished = plugin_message("relais run run-65abc-1f2e finished: needs_decision (x)");
        let waiting = flight_after(vec![finished.clone()]);
        assert_eq!(
            waiting.waiting,
            vec![("run-65abc-1f2e".to_string(), "needs_decision".to_string())]
        );
        assert!(flight_after(vec![finished.clone(), decide("run-other")]).in_flight());
        assert!(!flight_after(vec![finished, decide("run-65abc-1f2e")]).in_flight());
    }

    #[test]
    fn a_cancelled_run_that_launches_a_successor_stays_in_flight() {
        let successor = plugin_message(
            "relais run run-65abc-1f2e finished: cancelled (scope_widened_for_tests). relais launches successor intent x.",
        );
        assert!(flight_after(vec![successor.clone()]).in_flight());
        let exited = plugin_message("relais exited with code 9 without an outcome.");
        assert!(!flight_after(vec![successor, exited]).in_flight());
    }

    #[test]
    fn a_refused_start_is_not_in_flight() {
        let entries = vec![
            prompt("p"),
            tool_use("r1", "mcp__relais__run"),
            tool_result("r1", "Nothing ran: the scope is empty."),
        ];
        assert!(!relais_flight(&entries).in_flight());
    }

    #[test]
    fn a_started_text_from_another_tool_does_not_count() {
        let entries = vec![tool_use("b", "Bash"), tool_result("b", STARTED)];
        assert!(!relais_flight(&entries).in_flight());
    }

    #[test]
    fn a_plans_only_unpushed_commit_with_a_non_ascii_path_is_clean() {
        let repo = TestRepo::new();
        repo.commit_file("README.md", "x", "init");
        repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repo.commit_file("docs/plans/é.md", "plan", "docs(plan): é");
        match check_repositories(Some(repo.path()), &[], deadline()) {
            Repositories::Clean { heads } => {
                assert_eq!(heads.len(), 1);
                assert_eq!(heads[0].1.len(), 40);
            }
            other => panic!("expected clean, got {other:?}"),
        }
    }

    #[test]
    fn a_dirty_tree_is_not_clean() {
        let repo = TestRepo::new();
        repo.commit_file("README.md", "x", "init");
        repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repo.write("scratch.txt", "dirty");
        assert!(matches!(
            check_repositories(Some(repo.path()), &[], deadline()),
            Repositories::NotClean { .. }
        ));
    }

    #[test]
    fn an_unpushed_non_plan_commit_is_not_clean() {
        let repo = TestRepo::new();
        repo.commit_file("README.md", "x", "init");
        repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repo.commit_file("src/lib.rs", "fn f() {}", "feat: f");
        assert!(matches!(
            check_repositories(Some(repo.path()), &[], deadline()),
            Repositories::NotClean { .. }
        ));
    }

    #[test]
    fn a_missing_directory_is_clean_and_a_passed_deadline_is_timed_out() {
        let missing = PathBuf::from("/nonexistent/removed-worktree");
        assert_eq!(
            check_repositories(None, &[missing], deadline()),
            Repositories::Clean { heads: vec![] }
        );
        let repo = TestRepo::new();
        repo.commit_file("README.md", "x", "init");
        assert_eq!(
            check_repositories(Some(repo.path()), &[], Instant::now()),
            Repositories::TimedOut
        );
    }

    #[test]
    fn the_key_is_the_command_and_each_head() {
        let heads = vec![(PathBuf::from("/r"), "abc".to_string())];
        assert_eq!(boundary_key("cmd", &heads), "cmd\n/r abc");
        assert_ne!(
            boundary_key("cmd", &heads),
            boundary_key("cmd", &[(PathBuf::from("/r"), "def".to_string())])
        );
    }

    #[test]
    fn injected_prompts_are_no_person_prompts_but_still_open_a_turn() {
        let notification = json!({"type": "user", "uuid": "n", "message": {"role": "user",
            "content": "<task-notification>\nx</task-notification>"}});
        let framed = json!({"type": "user", "uuid": "m", "message": {"role": "user",
            "content": [{"type": "text", "text": "The relais plugin sent a message:\nhi"}]}});
        assert!(is_injected(&notification) && is_injected(&framed));
        assert!(!is_person_prompt(&notification) && !is_person_prompt(&framed));
        assert!(is_person_prompt(&prompt("p")) && !is_injected(&prompt("p")));
        let entries = vec![prompt("p"), notification];
        assert_eq!(turn_window(&entries).expect("a window").prompt_uuid, "n");
    }

    #[test]
    fn the_state_reads_both_formats_and_round_trips_a_pending_boundary() {
        let old = parse_state("t1 2\nkey\nmore").expect("old format");
        assert_eq!(
            (old.turn.as_str(), old.advices, old.pending),
            ("t1", 2, None)
        );
        assert_eq!(old.key, "key\nmore");
        let older = parse_state("t1\nk").expect("older format");
        assert_eq!((older.advices, older.key.as_str()), (0, "k"));
        let state = State {
            turn: "t".into(),
            key: "k\nv".into(),
            advices: 1,
            pending: Some(Pending {
                uuid: "u".into(),
                command: "git worktree remove x".into(),
                what: "worktree remove".into(),
                dirs: vec![PathBuf::from("/r")],
                unresolved: true,
            }),
        };
        assert_eq!(parse_state(&render_state(&state)), Some(state));
    }

    fn with_uuid(mut entry: Value, uuid: &str) -> Value {
        entry["uuid"] = json!(uuid);
        entry
    }

    /// A turn that reached a `worktree remove` boundary at 250k tokens.
    fn boundary_entries() -> Vec<Value> {
        vec![
            prompt("p1"),
            with_uuid(
                assistant(
                    Some((250_000, 0, 0)),
                    false,
                    json!([{"type": "tool_use", "id": "b1", "name": "Bash",
                            "input": {"command": "git worktree remove /tmp/wt"}}]),
                ),
                "boundary-entry",
            ),
        ]
    }

    fn fixture(entries: &[Value]) -> Value {
        json!({
            "payload": {"cwd": "/work", "permission_mode": "bypassPermissions",
                        "background_busy": false, "stop_hook_active": false,
                        "ends_with_question": false, "waiting_line": false,
                        "marker": false},
            "state": {"key": "", "advices": 0, "pending": null},
            "facts": {"phase": "none", "repositories": "clean"},
            "entries": entries,
        })
    }

    fn replay(fixture: &Value) -> Result<bool, FixtureError> {
        judge_case(&fixture.to_string())
    }

    #[test]
    fn a_fixture_with_a_boundary_and_nothing_in_flight_fires() {
        assert_eq!(replay(&fixture(&boundary_entries())), Ok(true));
        assert_eq!(
            stop_judge(
                "clear-at-boundary",
                &fixture(&boundary_entries()).to_string()
            ),
            Some(Ok(true))
        );
        assert_eq!(stop_judge("other-rule", "{}"), None);
    }

    #[test]
    fn a_fixture_with_a_frozen_pending_boundary_and_work_after_it_is_silent() {
        let mut entries = boundary_entries();
        entries.push(with_uuid(tool_use("w1", "Edit"), "work-entry"));
        let mut case = fixture(&entries);
        case["state"]["pending"] = json!({
            "uuid": "boundary-entry", "command": "git worktree remove /tmp/wt",
            "what": "worktree remove", "dirs": []
        });
        assert_eq!(replay(&case), Ok(false));
    }

    #[test]
    fn a_fixture_with_a_relais_run_in_flight_is_silent() {
        let mut entries = boundary_entries();
        entries.push(tool_use("r1", "mcp__relais__run"));
        entries.push(tool_result("r1", "Started relais run run-1"));
        assert_eq!(replay(&fixture(&entries)), Ok(false));
    }

    #[test]
    fn a_fixture_with_background_work_is_silent() {
        let mut case = fixture(&boundary_entries());
        case["payload"]["background_busy"] = json!(true);
        assert_eq!(replay(&case), Ok(false));
    }

    #[test]
    fn a_malformed_fixture_is_an_error() {
        assert!(matches!(
            judge_case("not json"),
            Err(FixtureError::NotJson(_))
        ));
        assert_eq!(
            judge_case("{}"),
            Err(FixtureError::Missing("payload".to_string()))
        );
        let mut case = fixture(&boundary_entries());
        case["facts"]["phase"] = json!("sideways");
        assert!(matches!(replay(&case), Err(FixtureError::WrongType(_))));
        let mut case = fixture(&boundary_entries());
        case["countable"] = json!(false);
        case["excluded"] = json!("why not");
        assert_eq!(
            replay(&case),
            Err(FixtureError::NotCountable("why not".to_string()))
        );
    }

    #[test]
    fn a_not_clean_repository_fact_decodes() {
        let mut case = fixture(&boundary_entries());
        case["facts"]["repositories"] = json!({"not_clean": "dirty"});
        assert_eq!(replay(&case), Ok(false));
    }

    #[test]
    fn an_unwritable_state_is_recorded_first_and_advises_nothing() {
        let entries = boundary_entries();
        let case = fixture(&entries);
        let view = StopView {
            cwd: Some(PathBuf::from("/work")),
            ..StopView::default()
        };
        let verdict = super::decide(
            &view,
            &entries,
            State::default(),
            || PhaseFact::NoOpenPhase,
            || Stance::Deny,
            |_| Repositories::Clean { heads: Vec::new() },
            |_| false,
        );
        assert!(matches!(verdict.decision, Decision::Silent));
        assert_eq!(
            verdict.records.first(),
            Some(&("watched".to_string(), "state-unwritable".to_string()))
        );
        assert_eq!(verdict.records.len(), 1);
        assert_eq!(
            replay(&case),
            Ok(true),
            "the same input fires when the write holds"
        );
    }

    #[test]
    fn the_uuid_suffix_survives_the_journal_trim() {
        let what = "x".repeat(400);
        let line = excerpt(&what, TailCut::Capped, Some("0123-uuid"));
        let trimmed = journal::redact_and_trim(&line);
        assert!(
            trimmed.ends_with(" tail-capped uuid=0123-uuid"),
            "got {trimmed}"
        );
        assert_eq!(excerpt("short", TailCut::Whole, None), "short");
    }

    #[test]
    fn a_tail_read_up_to_an_offset_stops_there() {
        let directory = scratch("at");
        let path = directory.join("t.jsonl");
        let entries = vec![prompt("one"), prompt("two"), prompt("three")];
        write_lines(&path, &entries);
        let first_two: u64 = entries[..2]
            .iter()
            .map(|e| format!("{e}\n").len() as u64)
            .sum();
        let tail = read_tail_until_at(&path, first_two, 64, 1 << 20, |_| false).expect("read");
        let uuids: Vec<_> = tail
            .entries
            .iter()
            .filter_map(|e| e["uuid"].as_str())
            .collect();
        assert_eq!(uuids, ["one", "two"]);
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// The one directory of the boundary `git -C <word> worktree remove x`.
    fn one_dir(word: &str, cwd: &str, home: Option<&str>) -> PathBuf {
        let dirs = dirs_with_home(&format!("git -C {word} worktree remove x"), cwd, home);
        assert_eq!(dirs.len(), 1, "{word}: {dirs:?}");
        dirs.into_iter().next().expect("one directory")
    }

    #[test]
    fn the_8145b6c0_boundary_names_the_chezmoi_source() {
        let text = include_str!(
            "../../greenbelay-guard/tests/corpus/stop/clear-at-boundary/8145b6c0-1794667260.json"
        );
        let fixture: Value = serde_json::from_str(text).expect("the fixture parses");
        let entries = fixture["entries"].as_array().expect("entries").clone();
        let window = turn_window(&entries).expect("a window");
        let found = boundaries(
            &window,
            Path::new("~/Developer/Perso"),
            Some(Path::new("~")),
        );
        let last = found.last().expect("a boundary");
        assert_eq!(last.what, "worktree remove");
        assert_eq!(last.dirs, [PathBuf::from("~/.local/share/chezmoi")]);
    }

    #[test]
    fn a_home_word_resolves_as_the_shell_expands_it() {
        for (word, expected) in [
            ("~", "/h"),
            ("~/a", "/h/a"),
            ("$HOME", "/h"),
            ("$HOME/a", "/h/a"),
            ("${HOME}/a", "/h/a"),
            ("\"$HOME/a\"", "/h/a"),
            ("\"$HOME\"/a", "/h/a"),
            ("~/a/../b", "/h/b"),
        ] {
            assert_eq!(
                one_dir(word, "/work", Some("/h")),
                PathBuf::from(expected),
                "{word}"
            );
        }
    }

    #[test]
    fn a_word_the_shell_does_not_expand_keeps_the_cwd_join() {
        // Checked against zsh 5.9 (`HO=x; print -r -- <word>`): none of these
        // is a path under home. `$HOMEX/a` is `/a`, `${HOME}x` is home's
        // sibling `/hx`, `$HO"ME"/a` is `xME/a`; the rule keeps each one as
        // written, as it keeps every word it does not resolve.
        for (word, expected) in [
            ("~user/a", "/work/~user/a"),
            ("~+", "/work/~+"),
            ("~-", "/work/~-"),
            ("$OTHER/a", "/work/$OTHER/a"),
            ("$HOMEX/a", "/work/$HOMEX/a"),
            ("'${HOME}x'", "/work/${HOME}x"),
            ("${HOME}x", "/work/${HOME}x"),
            ("\"~/a\"", "/work/~/a"),
            ("'~/a'", "/work/~/a"),
            ("'$HOME/a'", "/work/$HOME/a"),
            ("\\~/a", "/work/~/a"),
            ("\"\\$HOME/a\"", "/work/$HOME/a"),
            ("$HO\"ME\"/a", "/work/$HOME/a"),
        ] {
            assert_eq!(
                one_dir(word, "/work", Some("/h")),
                PathBuf::from(expected),
                "{word}"
            );
        }
    }

    #[test]
    fn without_a_home_a_tilde_word_is_joined_as_before() {
        assert_eq!(one_dir("~/a", "/work", None), PathBuf::from("/work/~/a"));
    }

    #[test]
    fn a_home_word_replaces_the_chain_built_so_far() {
        let chained = |words: &str, cwd: &str, home: &str| {
            dirs_with_home(&format!("git {words} worktree remove x"), cwd, Some(home))
        };
        assert_eq!(
            chained("-C ~/a -C ../b", "/work", "/h"),
            [PathBuf::from("/h/b")]
        );
        assert_eq!(
            chained("-C rel -C ~/a", "/work", "/h"),
            [PathBuf::from("/h/a")]
        );
        assert_eq!(
            chained("-C ~ -C rel", "/work", "/h"),
            [PathBuf::from("/h/rel")]
        );
        // The replay's symbolic home: the same answers, written from `~`.
        let cwd = "~/Developer/Perso";
        assert_eq!(chained("-C ~/a -C ../b", cwd, "~"), [PathBuf::from("~/b")]);
        assert_eq!(chained("-C rel -C ~/a", cwd, "~"), [PathBuf::from("~/a")]);
        // Above the symbolic home a `..` pops `~` itself: an empty path, which
        // the check skips. Live, the same word climbs to home's parent.
        assert_eq!(chained("-C ~/..", cwd, "~"), [PathBuf::new()]);
        assert_eq!(chained("-C ~ -C ..", cwd, "~"), [PathBuf::new()]);
        assert_eq!(
            chained("-C ~/..", cwd, "/Users/me"),
            [PathBuf::from("/Users")]
        );
    }

    #[test]
    fn only_an_absolute_home_is_read() {
        assert_eq!(home_from(None), None);
        assert_eq!(home_from(Some("".into())), None);
        assert_eq!(home_from(Some("relative/home".into())), None);
        let absolute = std::env::temp_dir();
        assert_eq!(home_from(Some(absolute.clone().into())), Some(absolute));
    }

    #[test]
    fn a_dirty_repository_under_home_is_not_clean() {
        let repo = TestRepo::new();
        repo.commit_file("f", "one\n", "init");
        let home = repo
            .path()
            .parent()
            .expect("a parent")
            .to_string_lossy()
            .into_owned();
        let name = repo
            .path()
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let dirs = dirs_with_home(
            &format!("git -C ~/{name} worktree remove x"),
            "/nowhere",
            Some(&home),
        );
        assert_eq!(dirs, [repo.path().to_path_buf()]);
        repo.write("f", "two\n");
        assert!(matches!(
            check_repositories(None, &dirs, deadline()),
            Repositories::NotClean { reason, .. } if reason == "uncommitted changes"
        ));
    }

    fn home_boundary_entries() -> Vec<Value> {
        vec![
            prompt("p1"),
            with_uuid(
                assistant(
                    Some((250_000, 0, 0)),
                    false,
                    json!([{"type": "tool_use", "id": "b1", "name": "Bash",
                            "input": {"command": "git -C ~/repo worktree remove x"}}]),
                ),
                "home-boundary",
            ),
        ]
    }

    #[test]
    fn a_per_directory_fact_answers_the_resolved_directory() {
        let mut case = fixture(&home_boundary_entries());
        case["facts"]["repositories"] = json!({"per_directory": {"~/repo": "uncommitted changes"}});
        assert_eq!(
            replay(&case),
            Ok(false),
            "held: the resolved directory is dirty"
        );
        case["facts"]["repositories"] = json!({"per_directory": {"~/repo": "timed_out"}});
        assert_eq!(replay(&case), Ok(false), "a timeout is silence");
        case["facts"]["repositories"] = json!({"per_directory": {"~/repo": "clean"}});
        assert_eq!(replay(&case), Ok(true));
        // The path the rule resolved before home was read: not asked for any
        // more, so the case fires. It passes only through the expansion.
        case["facts"]["repositories"] =
            json!({"per_directory": {"/work/~/repo": "uncommitted changes"}});
        assert_eq!(replay(&case), Ok(true));
        case["facts"]["repositories"] = json!({"per_directory": {"/work": "uncommitted changes"}});
        assert_eq!(replay(&case), Ok(false), "the payload cwd is walked first");
    }

    #[test]
    fn a_per_directory_fact_is_checked_as_it_loads() {
        let mut case = fixture(&home_boundary_entries());
        for (map, wrong) in [
            (json!({"repo": "dirty"}), "neither rooted nor under ~"),
            (json!({"~/repo": ""}), "is not clean, timed_out"),
            (json!({"~/repo": "clena"}), "is not clean, timed_out"),
        ] {
            case["facts"]["repositories"] = json!({ "per_directory": map });
            assert!(
                matches!(replay(&case), Err(FixtureError::WrongType(ref why)) if why.contains(wrong)),
                "{map}"
            );
        }
    }

    #[test]
    fn a_live_stop_view_reads_home_from_the_environment() {
        let stop = Stop {
            session: "home-session".to_string(),
            cwd: Some(PathBuf::from("/work")),
            permission_mode: None,
            last_message: None,
            background_busy: false,
            stop_hook_active: false,
            transcript: None,
        };
        assert_eq!(
            StopView::of(&stop).home,
            home_from(std::env::var_os("HOME"))
        );
    }

    #[test]
    fn a_pending_saved_before_home_was_read_is_checked_as_saved() {
        let entries = home_boundary_entries();
        let prior = State {
            pending: Some(Pending {
                uuid: "home-boundary".to_string(),
                command: "git -C ~/repo worktree remove x".to_string(),
                what: "worktree remove".to_string(),
                dirs: vec![PathBuf::from("/work/~/repo")],
                unresolved: false,
            }),
            ..State::default()
        };
        let view = StopView {
            cwd: Some(PathBuf::from("/work")),
            home: Some(PathBuf::from("/h")),
            ..StopView::default()
        };
        let asked = std::cell::RefCell::new(Vec::new());
        super::decide(
            &view,
            &entries,
            prior,
            || PhaseFact::NoOpenPhase,
            || Stance::Deny,
            |dirs| {
                asked.borrow_mut().extend(dirs.iter().cloned());
                Repositories::Clean { heads: Vec::new() }
            },
            |_| true,
        );
        // One resume with the old joined path; the next person prompt clears
        // the pending (see `judge`), so no migration is needed.
        assert_eq!(*asked.borrow(), [PathBuf::from("/work/~/repo")]);
    }

    #[test]
    fn a_state_file_without_the_unresolved_key_reads_as_resolved() {
        let text = r#"{"turn":"t","advices":1,"key":"k","pending":{"uuid":"u","command":"git worktree remove x","what":"worktree remove","dirs":["/r"]}}"#;
        let state = parse_state(text).expect("a state");
        let pending = state.pending.expect("a pending boundary");
        assert!(!pending.unresolved);
        assert_eq!(pending.dirs, [PathBuf::from("/r")]);
    }

    #[test]
    fn a_not_clean_hold_names_a_repository_other_than_the_cwds() {
        let entries = vec![
            prompt("p1"),
            with_uuid(
                assistant(
                    Some((250_000, 0, 0)),
                    false,
                    json!([{"type": "tool_use", "id": "b1", "name": "Bash",
                            "input": {"command": "git -C /h/wt worktree remove x"}}]),
                ),
                "boundary-entry",
            ),
        ];
        let view = StopView {
            cwd: Some(PathBuf::from("/work")),
            home: Some(PathBuf::from("/h")),
            ..StopView::default()
        };
        let verdict_of = |of: NotCleanOf| {
            super::decide(
                &view,
                &entries,
                State::default(),
                || PhaseFact::NoOpenPhase,
                || Stance::Deny,
                |_| Repositories::NotClean {
                    directory: PathBuf::from("/h/wt"),
                    reason: "uncommitted changes".to_string(),
                    of,
                },
                |_| true,
            )
        };
        assert_eq!(
            verdict_of(NotCleanOf::NamedDirectory).records,
            [(
                "watched".to_string(),
                "pending uncommitted changes dir=wt".to_string()
            )]
        );
        assert_eq!(
            verdict_of(NotCleanOf::PayloadCwd).records,
            [(
                "watched".to_string(),
                "pending uncommitted changes".to_string()
            )]
        );
    }

    /// The 4526105a boundary command (session-boundaries t23): `LIVE` is set
    /// at the top, and the `if … fi` between never assigns it.
    const LIVE_4526105A: &str = r#"LIVE=~/Developer/Perso/decisions; git -C "$LIVE" fetch origin -q; DEF=$(git -C "$LIVE" symbolic-ref --short refs/remotes/origin/HEAD | sed 's#^origin/##'); GD=$(git -C "$LIVE" rev-parse --absolute-git-dir); if [ "$(git -C "$LIVE" branch --show-current)" != "$DEF" ]; then echo "live: on $(git -C "$LIVE" branch --show-current), not $DEF; left alone"; elif [ -n "$(git -C "$LIVE" status --porcelain --untracked-files=no)" ]; then echo "live: uncommitted changes; left alone"; elif [ -e "$GD/MERGE_HEAD" ] || [ -d "$GD/rebase-merge" ] || [ -d "$GD/rebase-apply" ]; then echo "live: merge/rebase in progress; left alone"; else git -C "$LIVE" merge --ff-only -q "origin/$DEF" && echo "live: fast-forwarded to $(git -C "$LIVE" rev-parse --short HEAD)" || echo "live: cannot fast-forward; left alone"; fi; git -C "$LIVE" worktree remove ../decisions-wt-t21 && echo "removed decisions-wt-t21" && git -C "$LIVE" branch -D docs/session-boundaries-t21"#;

    /// The dirs and the unresolved shape of each boundary in `command`, with
    /// the payload cwd `cwd`, home `/h`, and the Bash call's entry carrying
    /// `entry_cwd` (none when `None`).
    fn walked(
        command: &str,
        cwd: &str,
        entry_cwd: Option<&str>,
    ) -> Vec<(Vec<PathBuf>, Option<&'static str>)> {
        let mut call = bash("b0", command);
        if let Some(entry_cwd) = entry_cwd {
            call["cwd"] = json!(entry_cwd);
        }
        let entries = vec![prompt("p1"), call];
        let window = turn_window(&entries).expect("a window");
        boundaries(&window, Path::new(cwd), Some(Path::new("/h")))
            .into_iter()
            .map(|boundary| (boundary.dirs, boundary.unresolved.map(Shape::as_str)))
            .collect()
    }

    fn paths(dirs: &[&str]) -> Vec<PathBuf> {
        dirs.iter().map(PathBuf::from).collect()
    }

    /// The plan's Phase 1 table (cwd `/w`, home `/h`): a command that fixes
    /// its directory is read, one that does not keeps today's dirs and is
    /// unresolved.
    #[test]
    fn the_walk_follows_a_chained_cd_and_same_command_assignments() {
        let resolved: &[(&str, &[&str])] = &[
            ("cd /h/repo && git worktree remove ../x", &["/h/repo"]),
            ("cd ~/repo; git commit -m \"docs(plan): p\"", &["/h/repo"]),
            (
                "LIVE=~/repo; git -C \"$LIVE\" worktree remove x",
                &["/h/repo"],
            ),
            (LIVE_4526105A, &["/h/Developer/Perso/decisions"]),
            ("W=/h/a; cd \"$W\" && git worktree remove ../b", &["/h/a"]),
            (
                "R=/h; git -C \"$R/relais\" worktree remove x",
                &["/h/relais"],
            ),
            ("cd /h/a && git -C ~/b worktree remove x", &["/h/b"]),
            ("cd /h/a 2>&1 && git worktree remove ../b", &["/h/a"]),
            ("cd /h/a >/dev/null && git worktree remove ../b", &["/h/a"]),
            ("LIVE='$x'; git -C \"$LIVE\" worktree remove y", &["/w/$x"]),
            ("cd ./child && git worktree remove ../x", &["/w/child"]),
            (
                "cd /h/a; R=/h/b; git -C \"$R\" worktree remove x",
                &["/h/b"],
            ),
            (
                "X=$(cd /h/a; pwd); git -C /h/b worktree remove x",
                &["/h/b"],
            ),
            // Delta 1: an unbound name set after `&&` keeps its value.
            (
                "gh x && LIVE=~/repo && git -C \"$LIVE\" fetch; git -C \"$LIVE\" worktree remove x",
                &["/h/repo"],
            ),
        ];
        for (command, dirs) in resolved {
            assert_eq!(
                walked(command, "/w", Some("/w")),
                vec![(paths(dirs), None)],
                "{command}"
            );
        }
    }

    #[test]
    fn a_shape_the_walk_cannot_read_keeps_todays_dirs_and_is_unresolved() {
        let unresolved: &[(&str, &[&str], &str)] = &[
            (
                "git -C \"$LIVE\" worktree remove x",
                &["/w/$LIVE"],
                "unassigned",
            ),
            ("(cd /h/a && git worktree remove ../b)", &[], "subshell"),
            (
                "LIVE=$(git rev-parse --show-toplevel); git -C \"$LIVE\" worktree remove x",
                &["/w/$LIVE"],
                "command-subst",
            ),
            (
                "for r in a b; do LIVE=/h/$r; git -C \"$LIVE\" worktree remove x; done",
                &["/w/$LIVE"],
                "loop-var",
            ),
            ("cd - && git worktree remove x", &[], "cd-dash"),
            ("cd /h/a | cat; git worktree remove x", &[], "cd-piped"),
            ("cd /h/a || git worktree remove x", &[], "cd-or"),
            ("cd ~- && git worktree remove x", &[], "not-literal"),
            (
                "R=/a; false && R=/b; git -C \"$R\" worktree remove x",
                &["/w/$R"],
                "conditional",
            ),
            (
                "test -d /h/a || LIVE=/h/b; git -C \"$LIVE\" worktree remove x",
                &["/w/$LIVE"],
                "conditional",
            ),
        ];
        for (command, dirs, shape) in unresolved {
            assert_eq!(
                walked(command, "/w", Some("/w")),
                vec![(paths(dirs), Some(*shape))],
                "{command}"
            );
        }
        // Carried from review: CDPATH is invisible to the hook, so a relative
        // `cd` word without `./` or `../` is not read (unix B); a leading
        // `then`/`do`/`else`/`elif`/`{` inside a compound is stripped before
        // the walk reads the command, so its `cd` or assignment still counts
        // as made inside the compound (backend 7).
        let unread: &[(&str, &[&str])] = &[
            ("cd repo && git worktree remove ../x", &[]),
            ("if c; then cd /h/a; fi; git worktree remove x", &[]),
            (
                "if c; then LIVE=/h/b; fi; git -C \"$LIVE\" worktree remove x",
                &["/w/$LIVE"],
            ),
            (
                "R=/a; false && R=/b; git -C \"$R\" worktree remove x",
                &["/w/$R"],
            ),
        ];
        for (command, dirs) in unread {
            let found = walked(command, "/w", Some("/w"));
            assert_eq!(found.len(), 1, "{command}");
            assert_eq!(found[0].0, paths(dirs), "{command}");
            assert!(found[0].1.is_some(), "{command} is unresolved");
        }
    }

    /// The walk starts where the call began, the entry's cwd: the payload cwd
    /// can already be past the call's own `cd`. Without an entry cwd it
    /// starts at the payload cwd.
    #[test]
    fn the_walk_starts_at_the_entry_cwd_then_the_payload_cwd() {
        let command = "cd ./child && git worktree remove ../x";
        assert_eq!(
            walked(command, "/w/child", Some("/w")),
            vec![(paths(&["/w/child"]), None)]
        );
        assert_eq!(
            walked(command, "/w", None),
            vec![(paths(&["/w/child"]), None)]
        );
        // The call began in the `cd` target, but the Stop's cwd is elsewhere:
        // the target is still checked.
        assert_eq!(
            walked("cd /h/a && git worktree remove ../x", "/w", Some("/h/a")),
            vec![(paths(&["/h/a"]), None)]
        );
    }

    /// An unresolved own boundary adds one `watched unresolved <shape>
    /// <what>` record, first, and the decision is the one without it.
    #[test]
    fn an_unresolved_boundary_is_journalled_and_decides_the_same() {
        let entries_for = |command: &str| {
            vec![
                prompt("p1"),
                with_uuid(
                    assistant(
                        Some((250_000, 0, 0)),
                        false,
                        json!([{"type": "tool_use", "id": "b1", "name": "Bash",
                                "input": {"command": command}}]),
                    ),
                    "boundary-entry",
                ),
            ]
        };
        let view = StopView {
            cwd: Some(PathBuf::from("/work")),
            home: Some(PathBuf::from("/h")),
            ..StopView::default()
        };
        let verdict_for = |command: &str| {
            super::decide(
                &view,
                &entries_for(command),
                State::default(),
                || PhaseFact::NoOpenPhase,
                || Stance::Deny,
                |_| Repositories::Clean { heads: Vec::new() },
                |_| true,
            )
        };
        let unresolved = verdict_for("(cd /h/a && git worktree remove ../b)");
        let plain = verdict_for("git worktree remove ../b");
        assert_eq!(
            unresolved.records.first(),
            Some(&(
                "watched".to_string(),
                "unresolved subshell worktree remove".to_string()
            ))
        );
        assert_eq!(
            unresolved
                .records
                .iter()
                .filter(|(_, what)| what.starts_with("unresolved "))
                .count(),
            1
        );
        assert_eq!(unresolved.records[1..], plain.records[..]);
        assert!(matches!(unresolved.decision, Decision::Continue(_)));
        assert!(matches!(plain.decision, Decision::Continue(_)));
    }

    fn turn_of(commands: &[&str]) -> Vec<Value> {
        let mut entries = vec![prompt("p1")];
        for (at, command) in commands.iter().enumerate() {
            entries.push(with_uuid(
                assistant(
                    Some((250_000, 0, 0)),
                    false,
                    json!([{"type": "tool_use", "id": format!("b{at}"), "name": "Bash",
                            "input": {"command": command}}]),
                ),
                &format!("entry-{at}"),
            ));
        }
        entries
    }

    fn deny_view() -> StopView {
        StopView {
            cwd: Some(PathBuf::from("/work")),
            home: Some(PathBuf::from("/h")),
            ..StopView::default()
        }
    }

    fn unresolved_rows(verdict: &Verdict) -> Vec<String> {
        verdict
            .records
            .iter()
            .filter(|(_, what)| what.starts_with("unresolved "))
            .map(|(_, what)| what.clone())
            .collect()
    }

    #[test]
    fn a_boundary_judged_again_adds_no_second_unresolved_row() {
        let entries = turn_of(&["(cd /h/a && git worktree remove ../b)"]);
        let view = deny_view();
        let saved = std::cell::RefCell::new(None);
        let decide_with = |prior: State| {
            super::decide(
                &view,
                &entries,
                prior,
                || PhaseFact::NoOpenPhase,
                || Stance::Deny,
                |_| Repositories::Clean { heads: Vec::new() },
                |state| {
                    *saved.borrow_mut() = Some(state.clone());
                    true
                },
            )
        };
        let first = decide_with(State::default());
        let prior = saved
            .borrow()
            .clone()
            .expect("the first Stop saved its key");
        let second = decide_with(prior);
        assert_eq!(unresolved_rows(&first).len(), 1);
        assert_eq!(unresolved_rows(&second).len(), 0);
    }

    #[test]
    fn the_unresolved_row_names_the_last_boundary_only() {
        let unresolved_then_plain = turn_of(&[
            "(cd /h/a && git worktree remove ../b)",
            "git worktree remove ../c",
        ]);
        let plain_then_unresolved = turn_of(&[
            "git worktree remove ../c",
            "(cd /h/a && git worktree remove ../b)",
        ]);
        let verdict_for = |entries: &[Value]| {
            super::decide(
                &deny_view(),
                entries,
                State::default(),
                || PhaseFact::NoOpenPhase,
                || Stance::Deny,
                |_| Repositories::Clean { heads: Vec::new() },
                |_| true,
            )
        };
        assert!(unresolved_rows(&verdict_for(&unresolved_then_plain)).is_empty());
        assert_eq!(
            unresolved_rows(&verdict_for(&plain_then_unresolved)),
            ["unresolved subshell worktree remove"]
        );
    }

    #[test]
    fn the_walk_starts_at_a_usable_entry_cwd_only() {
        let command = "git worktree remove x";
        let dirs_with = |entry_cwd: &str, home: &str| {
            let mut call = bash("b0", command);
            call["cwd"] = json!(entry_cwd);
            let entries = vec![prompt("p1"), call];
            let window = turn_window(&entries).expect("a window");
            boundaries(&window, Path::new("/w"), Some(Path::new(home)))
                .into_iter()
                .map(|boundary| boundary.dirs)
                .collect::<Vec<_>>()
        };
        assert_eq!(dirs_with("/other", "/h"), vec![paths(&["/other"])]);
        assert_eq!(dirs_with("", "/h"), vec![Vec::<PathBuf>::new()]);
        assert_eq!(dirs_with("relative/dir", "/h"), vec![Vec::<PathBuf>::new()]);
        assert_eq!(dirs_with("~/work", "~"), vec![paths(&["~/work"])]);
        assert_eq!(dirs_with("~/work", "/h"), vec![Vec::<PathBuf>::new()]);
    }

    #[test]
    fn a_shell_comment_is_not_a_command() {
        assert_eq!(
            walked("# cd /h/a\ngit worktree remove ../b", "/w", Some("/w")),
            vec![(Vec::new(), None)]
        );
        assert_eq!(
            walked("# git worktree remove ../b", "/w", Some("/w")),
            Vec::new()
        );
    }

    #[test]
    fn a_command_that_moves_the_shell_unseen_leaves_a_later_boundary_unresolved() {
        for word in [
            "pushd /h/a",
            "popd",
            "builtin cd /h/a",
            "command cd /h/a",
            "eval 'cd /h/a'",
            "source ./env",
            ". ./env",
        ] {
            assert_eq!(
                walked(
                    &format!("{word} && git worktree remove ../b"),
                    "/w",
                    Some("/w")
                ),
                vec![(Vec::new(), Some("not-literal"))],
                "{word}"
            );
        }
    }

    #[test]
    fn a_heredoc_body_is_read_raw() {
        let command = "cat > f <<EOF\ndon't\nEOF\ncd /h/a && git worktree remove ../b";
        assert_eq!(
            walked(command, "/w", Some("/w")),
            vec![(paths(&["/h/a"]), None)]
        );
    }

    #[test]
    fn a_cd_followed_by_exit_keeps_its_target() {
        for tail in ["exit", "exit 1", "return", "return 2"] {
            assert_eq!(
                walked(
                    &format!("cd /h/a || {tail}; git worktree remove ../b"),
                    "/w",
                    Some("/w")
                ),
                vec![(paths(&["/h/a"]), None)],
                "{tail}"
            );
        }
        assert_eq!(
            walked("cd /h/a || git worktree remove x", "/w", Some("/w")),
            vec![(Vec::new(), Some("cd-or"))]
        );
        assert_eq!(
            walked("cd /h/a || exit && git worktree remove x", "/w", Some("/w")),
            vec![(Vec::new(), Some("cd-or"))]
        );
    }

    #[test]
    fn a_while_or_until_compound_is_a_keyword_shape() {
        for head in ["while", "until"] {
            assert_eq!(
                walked(
                    &format!("{head} c; do LIVE=/h/a; git -C \"$LIVE\" worktree remove x; done"),
                    "/w",
                    Some("/w")
                ),
                vec![(paths(&["/w/$LIVE"]), Some("keyword"))],
                "{head}"
            );
        }
    }

    /// The plan's Behaviour table (t25), payload and entry cwd `/w`, home
    /// `/h` (as [`walked`] sets it): a boundary led
    /// by a reserved word is judged like any other.
    #[test]
    fn a_boundary_led_by_a_keyword_is_a_boundary() {
        type Walked = Vec<(Vec<PathBuf>, Option<&'static str>)>;
        let cases: &[(&str, Walked)] = &[
            (
                "for r in a b; do git -C $r worktree remove x; done",
                vec![(paths(&["/w/$r"]), Some("loop-var"))],
            ),
            (
                "H=/h/repo; for w in a; do git -C $H worktree remove $w; done",
                vec![(paths(&["/h/repo"]), None)],
            ),
            (
                "LIVE=/h/r; if true; then git -C \"$LIVE\" worktree remove x; fi",
                vec![(paths(&["/h/r"]), None)],
            ),
            (
                "if true; then :; else git -C /h/e worktree remove x; fi",
                vec![(paths(&["/h/e"]), None)],
            ),
            (
                "{ git commit -m \"docs(plan): x\"; }",
                vec![(Vec::new(), None)],
            ),
            (
                "if true; then git commit -m \"docs(plan): x, done\"; fi",
                Vec::new(),
            ),
            ("echo \"then git worktree remove x\"", Vec::new()),
            ("\"do\" git worktree remove x", Vec::new()),
        ];
        for (command, expected) in cases {
            assert_eq!(&walked(command, "/w", Some("/w")), expected, "{command}");
        }
    }

    /// Every word the walk strips before a simple command, `classify` strips
    /// too.
    #[test]
    fn classify_and_the_walk_strip_the_same_keywords() {
        for word in CONTINUING_KEYWORDS.iter().chain(OPENING_KEYWORDS.iter()) {
            let command = format!("{word} git -C /h/a worktree remove x");
            assert_eq!(
                walked(&command, "/w", Some("/w")),
                vec![(paths(&["/h/a"]), None)],
                "{command}"
            );
        }
        // The walk reads the `cd` after the word: after a continuing word it
        // moves the shell, inside a compound a word opens it is `keyword`.
        let cd_after = |word: &str| {
            walked(
                &format!("{word} cd /h/b; git worktree remove x"),
                "/w",
                Some("/w"),
            )
        };
        for word in CONTINUING_KEYWORDS {
            assert_eq!(cd_after(word), vec![(paths(&["/h/b"]), None)], "{word}");
        }
        for word in OPENING_KEYWORDS {
            assert_eq!(
                cd_after(word),
                vec![(Vec::new(), Some("keyword"))],
                "{word}"
            );
        }
    }

    #[test]
    fn a_keyword_led_heredoc_line_reads_as_a_bare_one() {
        let bare = walked(
            "cat > s.sh <<EOF\ngit worktree remove x\nEOF",
            "/w",
            Some("/w"),
        );
        let led = walked(
            "cat > s.sh <<EOF\ndo git worktree remove x\nEOF",
            "/w",
            Some("/w"),
        );
        assert_eq!(led, bare);
        assert_eq!(led.len(), 1);
    }

    #[test]
    fn command_and_builtin_unread_only_a_command_that_moves_the_shell() {
        let resolved = [
            "command -v jq && git worktree remove ../x",
            "builtin echo hi; git worktree remove ../x",
            "command git status && git worktree remove ../x",
        ];
        for command in resolved {
            assert_eq!(
                walked(command, "/w", Some("/w")),
                vec![(Vec::new(), None)],
                "{command}"
            );
        }
        let unread = [
            "command cd /h/b && git worktree remove x",
            "command -v cd && git worktree remove x",
            "command -- cd /h/b && git worktree remove x",
            "command -p builtin cd /h/b && git worktree remove x",
            "builtin source env.sh; git worktree remove x",
            "command eval 'cd /h/b'; git worktree remove x",
            "command \"$c\" /h/b && git worktree remove x",
            "builtin . ./env; git worktree remove x",
        ];
        for command in unread {
            assert_eq!(
                walked(command, "/w", Some("/w")),
                vec![(Vec::new(), Some("not-literal"))],
                "{command}"
            );
        }
    }

    fn rows_named(verdict: &Verdict, what: &str) -> usize {
        verdict
            .records
            .iter()
            .filter(|(_, row)| row == what)
            .count()
    }

    /// Two Stops on one unresolved boundary, past the advice cap, and the
    /// state the first one saved.
    fn two_stops_at_the_cap(entries: &[Value], second: &[Value]) -> (Verdict, Verdict) {
        let view = deny_view();
        let saved = std::cell::RefCell::new(None);
        let decide_with = |entries: &[Value], prior: State| {
            super::decide(
                &view,
                entries,
                prior,
                || PhaseFact::NoOpenPhase,
                || Stance::Deny,
                |_| Repositories::Clean { heads: Vec::new() },
                |state| {
                    *saved.borrow_mut() = Some(state.clone());
                    true
                },
            )
        };
        let capped = State {
            advices: MAX_ADVICES_PER_SESSION,
            ..State::default()
        };
        let first = decide_with(entries, capped.clone());
        let prior = saved.borrow().clone().unwrap_or(capped);
        let second = decide_with(second, prior);
        (first, second)
    }

    #[test]
    fn two_stops_at_the_cap_write_one_unresolved_row() {
        let entries = turn_of(&["for r in a; do git -C $r worktree remove x; done"]);
        let (first, second) = two_stops_at_the_cap(&entries, &entries);
        assert_eq!(
            unresolved_rows(&first),
            ["unresolved loop-var worktree remove"]
        );
        assert_eq!(rows_named(&first, "cap"), 1);
        assert!(unresolved_rows(&second).is_empty());
        assert_eq!(rows_named(&second, "cap"), 0);
    }

    #[test]
    fn two_boundaries_at_the_cap_write_two_cap_rows() {
        let first_turn = turn_of(&["git worktree remove ../a"]);
        let second_turn = turn_of(&["git worktree remove ../a", "git worktree remove ../b"]);
        let (first, second) = two_stops_at_the_cap(&first_turn, &second_turn);
        assert_eq!(rows_named(&first, "cap"), 1);
        assert_eq!(rows_named(&second, "cap"), 1);
    }
}
