//! The weekly roll-up of what the journal saw: `rollup.log`, beside the
//! journal.
//!
//! `journal::rotate` folds the generation it is about to delete into this file
//! first, so firing counts outlive the journal's ten generations. Like the
//! journal it only counts: a wrong read here miscounts and never blocks.
//!
//! Format, one line each, the header first:
//!
//! ```text
//! greenbelay-rollup-v1
//! S <ts>                                       oldest record the first fold saw
//! W <ts>                                       watermark: newest record folded
//! R <week> <rule> <stance> <outcome> <arm> <n> firings; week is the Monday
//! ```
//!
//! Any other line kind (the `C` calls rows of a later release, `L` rows) is
//! carried through a rewrite unchanged, so an older binary never drops them.
//! A header naming another version is skipped whole: nothing is counted from
//! it and nothing is overwritten.
//!
//! The file is rewritten whole under `rollup.log.lock` (temp file, fsync,
//! rename), never appended, so a crash leaves the old file or the new one.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::civil;
use crate::journal::{self, Arm};

pub const FORMAT: &str = "greenbelay-rollup-v1";

/// Weeks of `R` rows kept, newest first.
pub const WEEKS_KEPT: i64 = 104;

const LOCK_STALE_SECS: u64 = 60;
const SECONDS_PER_DAY: u64 = 86_400;
const DAYS_PER_WEEK: i64 = 7;

/// The roll-up that sits beside the journal at `journal_path`.
pub fn path_beside(journal_path: &Path) -> PathBuf {
    journal_path.with_file_name("rollup.log")
}

fn lock_path(rollup: &Path) -> PathBuf {
    let mut name = rollup.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

/// One counted bucket: a week's firings of one rule at one stance and outcome.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    /// The Monday of the UTC week.
    pub week: civil::Day,
    pub rule: String,
    pub stance: String,
    pub outcome: String,
    /// `live` or `shadow`; the arm stays in the key so a reader drops shadow.
    pub arm: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Rollup {
    /// `S`: the oldest record the first fold saw.
    pub covered_since: Option<u64>,
    /// `W`: the newest record timestamp folded so far.
    pub watermark: u64,
    pub rows: BTreeMap<Key, u64>,
    /// Lines of a kind this version does not know, kept in order.
    pub carried: Vec<String>,
    /// Records the last [`Rollup::fold_text`] left out for a timestamp past its
    /// ceiling. Not written to the file.
    pub dropped_future: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    Rollup(Rollup),
    /// The header named a version this build does not read.
    UnknownVersion(String),
}

/// Why a fold did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldError {
    /// `rollup.log.lock` is held by someone else.
    Busy,
    /// An I/O failure; the text is one whitespace-free token.
    Io(String),
}

impl FoldError {
    /// The reason as a journal field.
    pub fn why(&self) -> String {
        match self {
            FoldError::Busy => "lock-busy".to_string(),
            FoldError::Io(why) => why.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folded {
    Counted,
    /// The roll-up has another format version; nothing was written.
    SkippedUnknownVersion,
    /// Counted, but this many records were stamped more than a day ahead of the
    /// clock and left out, so a skewed clock cannot move the watermark.
    DroppedFuture(u64),
}

fn io_why(error: &std::io::Error) -> String {
    error.kind().to_string().replace(char::is_whitespace, "-")
}

/// Read `path`. A missing or empty file is an empty roll-up; an unreadable one
/// is an `Err` carrying the reason.
pub fn read(path: &Path) -> Result<Loaded, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Loaded::Rollup(Rollup::default()));
        }
        Err(e) => return Err(io_why(&e)),
    };
    Ok(parse(&text))
}

pub fn parse(text: &str) -> Loaded {
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Loaded::Rollup(Rollup::default());
    };
    if header != FORMAT {
        return Loaded::UnknownVersion(header.to_string());
    }
    let mut rollup = Rollup::default();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        if !rollup.take_known(line) {
            rollup.carried.push(line.to_string());
        }
    }
    Loaded::Rollup(rollup)
}

impl Rollup {
    /// Absorb a line of a kind this version knows; `false` for anything else.
    fn take_known(&mut self, line: &str) -> bool {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["S", ts] => ts.parse().map(|ts| self.covered_since = Some(ts)).is_ok(),
            ["W", ts] => ts.parse().map(|ts| self.watermark = ts).is_ok(),
            ["R", week, rule, stance, outcome, arm, count] => {
                let (Some(week), Ok(count)) = (civil::day_of(week), count.parse::<u64>()) else {
                    return false;
                };
                let key = Key {
                    week,
                    rule: (*rule).to_string(),
                    stance: (*stance).to_string(),
                    outcome: (*outcome).to_string(),
                    arm: (*arm).to_string(),
                };
                *self.rows.entry(key).or_default() += count;
                true
            }
            _ => false,
        }
    }

    pub fn render(&self) -> String {
        let mut out = format!("{FORMAT}\n");
        if let Some(ts) = self.covered_since {
            out.push_str(&format!("S {ts}\n"));
        }
        out.push_str(&format!("W {}\n", self.watermark));
        for (key, count) in &self.rows {
            out.push_str(&format!(
                "R {} {} {} {} {} {count}\n",
                civil::iso(key.week),
                key.rule,
                key.stance,
                key.outcome,
                key.arm
            ));
        }
        for line in &self.carried {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Count the journal records in `text` that are newer than the watermark
    /// this roll-up held when the fold began. Returns whether any record was
    /// seen at all. A record stamped after `ceiling` is a skewed clock's: it is
    /// counted in [`Rollup::dropped_future`] and moves neither the watermark nor
    /// the prune cutoff.
    pub fn fold_text(&mut self, text: &str, ceiling: u64) -> bool {
        let floor = self.watermark;
        let mut newest = None::<u64>;
        let mut oldest = None::<u64>;
        self.dropped_future = 0;
        for row in text.lines().filter_map(journal::fixed_fields) {
            if row.ts > ceiling {
                self.dropped_future += 1;
                continue;
            }
            newest = newest.max(Some(row.ts));
            oldest = Some(oldest.map_or(row.ts, |o| o.min(row.ts)));
            if row.ts <= floor {
                continue;
            }
            if !is_firing(&row.stance, &row.outcome) {
                continue;
            }
            let key = Key {
                week: civil::week_start(i64::try_from(row.ts / SECONDS_PER_DAY).unwrap_or(0)),
                rule: row.rule,
                stance: row.stance,
                outcome: row.outcome,
                arm: match row.arm {
                    Arm::Live => "live",
                    Arm::Shadow => "shadow",
                }
                .to_string(),
            };
            *self.rows.entry(key).or_default() += 1;
        }
        let (Some(newest), Some(oldest)) = (newest, oldest) else {
            return false;
        };
        self.watermark = self.watermark.max(newest);
        self.covered_since.get_or_insert(oldest);
        self.prune();
        true
    }

    /// Drop `R` rows older than [`WEEKS_KEPT`] weeks before the newest one.
    fn prune(&mut self) {
        let Some(newest) = self.rows.keys().map(|k| k.week).max() else {
            return;
        };
        let cutoff = newest - DAYS_PER_WEEK * (WEEKS_KEPT - 1);
        self.rows.retain(|key, _| key.week >= cutoff);
    }
}

/// The buckets `journal::tally` counts: a decision outcome, or an unconfirmed
/// or unverified stance (whose outcome field carries the reason). Notes such
/// as `rules-checked` and the `F` records about the journal itself are not
/// firings.
fn is_firing(stance: &str, outcome: &str) -> bool {
    matches!(stance, "unconfirmed" | "unverified")
        || matches!(outcome, "denied" | "advised" | "watched")
}

/// A held `rollup.log.lock`, released on drop.
struct Lock(PathBuf);

impl Lock {
    /// Try once. A lock older than a minute belongs to a crashed holder and is
    /// removed.
    fn try_take(path: &Path) -> Option<Lock> {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        if opts.open(path).is_err() {
            let stale = fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > LOCK_STALE_SECS);
            if !stale || fs::remove_file(path).is_err() || opts.open(path).is_err() {
                return None;
            }
        }
        Some(Lock(path.to_path_buf()))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0); // a lock file that is already gone is the goal
    }
}

/// Fold the journal generation at `generation` into the roll-up at `rollup`.
/// Tries the lock once and never waits. Counting each record once is the
/// watermark's job, so folding the same generation twice is a no-op.
pub fn fold_generation(generation: &Path, rollup: &Path) -> Result<Folded, FoldError> {
    let _lock = Lock::try_take(&lock_path(rollup)).ok_or(FoldError::Busy)?;
    let text = match fs::read_to_string(generation) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Folded::Counted),
        Err(e) => return Err(FoldError::Io(io_why(&e))),
    };
    let mut current = match read(rollup).map_err(FoldError::Io)? {
        Loaded::Rollup(current) => current,
        Loaded::UnknownVersion(_) => return Ok(Folded::SkippedUnknownVersion),
    };
    let ceiling = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(u64::MAX, |elapsed| {
            elapsed.as_secs().saturating_add(SECONDS_PER_DAY)
        }); // a clock before the epoch drops nothing
    if current.fold_text(&text, ceiling) {
        write_atomic(rollup, &current.render()).map_err(|e| FoldError::Io(io_why(&e)))?;
    }
    Ok(match current.dropped_future {
        0 => Folded::Counted,
        rows => Folded::DroppedFuture(rows),
    })
}

/// Write `contents` to a temp file beside `target`, fsync it, rename it over.
/// Leftover temps older than the lock's staleness are removed first; the
/// caller holds the lock.
fn write_atomic(target: &Path, contents: &str) -> std::io::Result<()> {
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    remove_stale_temps(target, &name);
    let temp_sequence = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = target.with_file_name(format!("{name}.new.{}.{temp_sequence}", std::process::id()));
    let written = (|| {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&temp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, target)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp); // the write error is what is reported
    }
    written
}

fn remove_stale_temps(target: &Path, name: &str) {
    let Some(dir) = target.parent() else { return };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let prefix = format!("{name}.new.");
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() > LOCK_STALE_SECS);
        if old && entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = fs::remove_file(entry.path()); // a temp already gone is the goal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("greenbelay-rollup-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir); // a missing scratch dir is the normal case
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// 2023-11-14 22:13:20 UTC, a Tuesday; its week starts Monday 2023-11-13.
    const TUESDAY: u64 = 1_700_000_000;
    const MONDAY_DAY: civil::Day = 19_674;

    fn generation_text(rows: &[(u64, &str, &str)]) -> String {
        let mut text = format!("{}\n", journal::FORMAT);
        for (ts, rule, arm) in rows {
            text.push_str(&format!(
                "F {ts} {rule} deny denied sess repo default {arm} x\n"
            ));
        }
        text
    }

    fn live_count(rollup: &Rollup, rule: &str) -> u64 {
        rollup
            .rows
            .iter()
            .filter(|(k, _)| k.rule == rule && k.arm == "live")
            .map(|(_, n)| n)
            .sum()
    }

    fn load(path: &Path) -> Rollup {
        match read(path).expect("readable") {
            Loaded::Rollup(r) => r,
            Loaded::UnknownVersion(v) => panic!("unknown version {v}"),
        }
    }

    #[test]
    fn a_fold_counts_each_record_once_across_rotations_and_a_refold() {
        let dir = scratch("once");
        let rollup = dir.join("rollup.log");
        let first = dir.join("first");
        let second = dir.join("second");
        fs::write(
            &first,
            generation_text(&[(TUESDAY, "r", "arm=live"), (TUESDAY + 1, "r", "arm=live")]),
        )
        .unwrap();
        fs::write(&second, generation_text(&[(TUESDAY + 5, "r", "arm=live")])).unwrap();

        fold_generation(&first, &rollup).expect("first");
        assert_eq!(live_count(&load(&rollup), "r"), 2);
        fold_generation(&second, &rollup).expect("second");
        assert_eq!(live_count(&load(&rollup), "r"), 3);
        // A crash between the roll-up rename and the journal rename folds the
        // same generations again.
        fold_generation(&first, &rollup).expect("refold first");
        fold_generation(&second, &rollup).expect("refold second");
        let after = load(&rollup);
        assert_eq!(live_count(&after, "r"), 3);
        assert_eq!(after.watermark, TUESDAY + 5);
        assert_eq!(after.covered_since, Some(TUESDAY));
        assert!(after.rows.keys().all(|k| k.week == MONDAY_DAY));
        assert!(!lock_path(&rollup).exists(), "the lock is released");
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn shadow_rows_do_not_reach_the_live_keys() {
        let dir = scratch("shadow");
        let rollup = dir.join("rollup.log");
        let gen = dir.join("gen");
        fs::write(
            &gen,
            generation_text(&[(TUESDAY, "r", "arm=live"), (TUESDAY + 1, "r", "arm=shadow")]),
        )
        .unwrap();
        fold_generation(&gen, &rollup).expect("fold");
        assert_eq!(live_count(&load(&rollup), "r"), 1);
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn unknown_line_kinds_survive_a_fold() {
        let dir = scratch("carry");
        let rollup = dir.join("rollup.log");
        let gen = dir.join("gen");
        fs::write(
            &rollup,
            format!("{FORMAT}\nW 5\nC 2023-11-13 Bash 41210\nL 9 lost\n"),
        )
        .unwrap();
        fs::write(&gen, generation_text(&[(TUESDAY, "r", "arm=live")])).unwrap();
        fold_generation(&gen, &rollup).expect("fold");
        let text = fs::read_to_string(&rollup).unwrap();
        assert!(text.contains("\nC 2023-11-13 Bash 41210\n"), "{text}");
        assert!(text.contains("\nL 9 lost\n"), "{text}");
        assert!(text.contains("R 2023-11-13 r deny denied live 1"), "{text}");
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn an_unknown_header_version_is_skipped_untouched() {
        let dir = scratch("version");
        let rollup = dir.join("rollup.log");
        let gen = dir.join("gen");
        let original = "greenbelay-rollup-v2\nR whatever\n";
        fs::write(&rollup, original).unwrap();
        fs::write(&gen, generation_text(&[(TUESDAY, "r", "arm=live")])).unwrap();
        assert_eq!(
            fold_generation(&gen, &rollup),
            Ok(Folded::SkippedUnknownVersion)
        );
        assert_eq!(fs::read_to_string(&rollup).unwrap(), original);
        assert_eq!(
            read(&rollup),
            Ok(Loaded::UnknownVersion("greenbelay-rollup-v2".to_string()))
        );
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn a_held_lock_is_busy() {
        let dir = scratch("lock");
        let rollup = dir.join("rollup.log");
        let gen = dir.join("gen");
        fs::write(&gen, generation_text(&[(TUESDAY, "r", "arm=live")])).unwrap();
        fs::write(lock_path(&rollup), "").unwrap();
        assert_eq!(fold_generation(&gen, &rollup), Err(FoldError::Busy));
        assert!(
            lock_path(&rollup).exists(),
            "a busy lock is not ours to remove"
        );
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn a_stale_lock_is_taken() {
        let dir = scratch("stale");
        let rollup = dir.join("rollup.log");
        let gen = dir.join("gen");
        fs::write(&gen, generation_text(&[(TUESDAY, "r", "arm=live")])).unwrap();
        let lock = fs::File::create(lock_path(&rollup)).unwrap();
        lock.set_modified(
            std::time::SystemTime::now() - std::time::Duration::from_secs(LOCK_STALE_SECS + 5),
        )
        .unwrap();
        assert_eq!(fold_generation(&gen, &rollup), Ok(Folded::Counted));
        assert_eq!(live_count(&load(&rollup), "r"), 1);
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn a_future_record_neither_counts_nor_moves_the_watermark() {
        let mut rollup = Rollup::default();
        let text = generation_text(&[(TUESDAY, "r", "arm=live"), (TUESDAY * 3, "r", "arm=live")]);
        assert!(rollup.fold_text(&text, TUESDAY + SECONDS_PER_DAY));
        assert_eq!(rollup.dropped_future, 1);
        assert_eq!(rollup.watermark, TUESDAY);
        assert_eq!(live_count(&rollup, "r"), 1);
    }

    #[test]
    fn an_unreadable_rollup_is_an_io_failure() {
        let dir = scratch("unreadable");
        let rollup = dir.join("rollup.log");
        fs::create_dir(&rollup).unwrap();
        let gen = dir.join("gen");
        fs::write(&gen, generation_text(&[(TUESDAY, "r", "arm=live")])).unwrap();
        assert!(matches!(
            fold_generation(&gen, &rollup),
            Err(FoldError::Io(_))
        ));
        let _ = fs::remove_dir_all(&dir); // scratch cleanup
    }

    #[test]
    fn only_a_hundred_and_four_weeks_are_kept() {
        let mut rollup = Rollup::default();
        let week = 7 * SECONDS_PER_DAY;
        let start = TUESDAY - 110 * week;
        let mut text = format!("{}\n", journal::FORMAT);
        for i in 0..=110 {
            text.push_str(&format!(
                "F {} r deny denied s repo default x\n",
                start + i * week
            ));
        }
        rollup.fold_text(&text, u64::MAX);
        let weeks: std::collections::BTreeSet<_> = rollup.rows.keys().map(|k| k.week).collect();
        assert_eq!(weeks.len(), 104);
    }
}
