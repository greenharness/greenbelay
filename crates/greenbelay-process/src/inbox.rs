//! The unfiled inbox lines a session opening at `startup` or `clear` is told
//! about.
//!
//! A session that files its inbox lines at its boundary leaves nothing here;
//! a session that was cleared first, or that died, leaves lines behind. They
//! live in `<git-common-dir>/greenharness/inbox.md`, and a session that is
//! filing them moves the file to `inbox.<id>.filing` first, so an abandoned
//! take sits beside the inbox. Nothing is moved or written by reading.
//!
//! The line grammar is `<YYYY-MM-DD> <session8> [due=YYYY-MM-DD] <text>`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use greenbelay_guard::civil::{self, Day};
use greenbelay_guard::rules::Stance;

/// A line this many whole days before today, or older, is an orphan.
const ORPHAN_AGE_DAYS: Day = 2;
/// A take this old has no session still filing it.
const TAKE_ABANDONED_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_ORPHANS_SHOWN: usize = 5;
const MAX_LINE_CHARS: usize = 200;
const UNKNOWN_SESSION: &str = "?";

/// What the scan found, and the text to print when the stance lets it speak.
#[derive(Debug, PartialEq, Eq)]
pub struct Notice {
    /// `None` under `observe`: counted and journalled, never said.
    pub text: Option<String>,
    pub orphans: usize,
    pub fresh: usize,
    pub being_filed: usize,
}

/// `None` when the source is not a fresh start, or nothing scanned has a
/// line. `today` is a local `YYYY-MM-DD`.
pub fn notice(cwd: &Path, source: &str, stance: Stance, today: &str) -> Option<Notice> {
    if !matches!(source, "startup" | "clear") {
        return None;
    }
    let today = civil::day_of(today)?;
    let inbox = inbox_path(cwd)?;
    let scan = scan(&inbox, today, SystemTime::now());
    let total = scan.orphans.len() + scan.fresh.len() + scan.being_filed;
    if total == 0 {
        return None;
    }
    let text = match stance {
        Stance::Observe => None,
        Stance::Advise | Stance::Deny => Some(phrase(&inbox, total, &scan)),
    };
    Some(Notice {
        text,
        orphans: scan.orphans.len(),
        fresh: scan.fresh.len(),
        being_filed: scan.being_filed,
    })
}

#[derive(Default)]
struct Scan {
    orphans: Vec<String>,
    /// The session id of each fresh line.
    fresh: Vec<String>,
    being_filed: usize,
}

fn inbox_path(cwd: &Path) -> Option<PathBuf> {
    if !cwd.is_dir() {
        return None;
    }
    let common = greenbelay_guard::git::stdout_in(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .filter(|s| !s.is_empty())?;
    Some(PathBuf::from(common).join("greenharness").join("inbox.md"))
}

fn scan(inbox: &Path, today: Day, now: SystemTime) -> Scan {
    let mut scan = Scan::default();
    if let Some(text) = read(inbox) {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match session_of_fresh(line, today) {
                Some(session) => scan.fresh.push(session),
                None => scan.orphans.push(line.to_string()),
            }
        }
    }
    let Some(dir) = inbox.parent() else {
        return scan;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return scan;
    };
    let mut takes: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("inbox.") && n.ends_with(".filing"))
        })
        .collect();
    takes.sort();
    for take in takes {
        let Some(text) = read(&take) else { continue };
        let age = std::fs::metadata(&take)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .unwrap_or_default();
        let lines = text.lines().filter(|l| !l.trim().is_empty());
        if age > TAKE_ABANDONED_AFTER {
            scan.orphans.extend(lines.map(str::to_string));
        } else {
            scan.being_filed += lines.count();
        }
    }
    scan
}

/// A missing file is no lines; any other read failure is silence too.
fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// The session id of a line that is not an orphan: dated under two days ago,
/// or malformed (session `?`). `None` is an orphan.
fn session_of_fresh(line: &str, today: Day) -> Option<String> {
    let mut words = line.split_whitespace();
    let date = words
        .next()
        .filter(|w| w.len() == 10)
        .and_then(civil::day_of);
    let session = words.next();
    match (date, session) {
        (Some(day), Some(session)) => (today - day < ORPHAN_AGE_DAYS).then(|| session.to_string()),
        _ => Some(UNKNOWN_SESSION.to_string()),
    }
}

fn phrase(inbox: &Path, total: usize, scan: &Scan) -> String {
    let mut out = format!(
        concat!(
            greenbelay_guard::bin!(),
            "/inbox: {} holds {} unfiled lines (nothing was moved):"
        ),
        inbox.display(),
        total
    );
    if !scan.orphans.is_empty() {
        out.push_str(&format!(
            "\n{} older than 2 days:",
            lines_of(scan.orphans.len())
        ));
        for line in scan.orphans.iter().take(MAX_ORPHANS_SHOWN) {
            out.push_str("\n- ");
            out.push_str(&cut(line));
        }
        if scan.orphans.len() > MAX_ORPHANS_SHOWN {
            out.push_str(&format!(
                "\n- … and {} more",
                scan.orphans.len() - MAX_ORPHANS_SHOWN
            ));
        }
        out.push_str("\nany session may file an orphan as a task ticket");
    }
    if scan.being_filed > 0 {
        out.push_str(&format!("\n{} being filed", lines_of(scan.being_filed)));
    }
    if !scan.fresh.is_empty() {
        let mut sessions: Vec<&str> = Vec::new();
        for session in &scan.fresh {
            if !sessions.contains(&session.as_str()) {
                sessions.push(session);
            }
        }
        out.push_str(&format!(
            "\n{} under 2 days old, of sessions {}: a live session files its own at its boundary; after a /clear, read {} and file the lines carrying the cleared session's session8",
            lines_of(scan.fresh.len()),
            sessions.join(", "),
            inbox.display()
        ));
    }
    out
}

fn lines_of(count: usize) -> String {
    if count == 1 {
        "1 line".to_string()
    } else {
        format!("{count} lines")
    }
}

fn cut(line: &str) -> String {
    if line.chars().count() <= MAX_LINE_CHARS {
        return line.to_string();
    }
    let mut cut: String = line.chars().take(MAX_LINE_CHARS).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> Day {
        civil::day_of("2026-11-17").unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("greenbelay-inbox-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn orphan(line: &str) -> bool {
        session_of_fresh(line, today()).is_none()
    }

    #[test]
    fn a_line_three_days_old_is_an_orphan() {
        assert!(orphan("2026-11-14 abcd1234 file the thing"));
    }

    #[test]
    fn two_days_is_the_boundary() {
        assert!(orphan("2026-11-15 abcd1234 x"));
        assert!(!orphan("2026-11-16 abcd1234 x"));
    }

    #[test]
    fn yesterday_and_today_are_fresh_with_their_session() {
        assert_eq!(
            session_of_fresh("2026-11-16 abcd1234 x", today()).as_deref(),
            Some("abcd1234")
        );
        assert_eq!(
            session_of_fresh("2026-11-17 feed0001 x", today()).as_deref(),
            Some("feed0001")
        );
    }

    #[test]
    fn a_malformed_line_is_fresh_under_an_unknown_session() {
        assert_eq!(
            session_of_fresh("no date here", today()).as_deref(),
            Some("?")
        );
        assert_eq!(
            session_of_fresh("2026-11-14", today()).as_deref(),
            Some("?")
        );
    }

    #[test]
    fn a_due_line_is_read_by_its_date_prefix() {
        assert!(orphan("2026-11-10 abcd1234 due=2026-12-01 later"));
        assert!(!orphan("2026-11-17 abcd1234 due=2026-11-10 now"));
    }

    #[test]
    fn an_old_take_is_all_orphans_and_a_young_one_is_counted_only() {
        let dir = scratch("takes");
        let take = dir.join("inbox.abc.filing");
        std::fs::write(&take, "2026-11-17 abcd1234 a\n2026-11-17 abcd1234 b\n").unwrap();
        let inbox = dir.join("inbox.md");
        let young = scan(&inbox, today(), SystemTime::now());
        assert_eq!((young.orphans.len(), young.being_filed), (0, 2));
        let later = SystemTime::now() + Duration::from_secs(25 * 60 * 60);
        let old = scan(&inbox, today(), later);
        assert_eq!((old.orphans.len(), old.being_filed), (2, 0));
    }

    #[test]
    fn a_take_is_read_beside_an_empty_or_missing_inbox() {
        let dir = scratch("empty");
        std::fs::write(dir.join("inbox.md"), "").unwrap();
        std::fs::write(dir.join("inbox.x.filing"), "2026-11-10 abcd1234 a\n").unwrap();
        let later = SystemTime::now() + Duration::from_secs(25 * 60 * 60);
        assert_eq!(scan(&dir.join("inbox.md"), today(), later).orphans.len(), 1);
    }

    #[test]
    fn the_text_cuts_by_chars_and_names_the_overflow() {
        let long = format!("2026-11-01 abcd1234 {}", "é".repeat(300));
        let scan = Scan {
            orphans: vec![long; 7],
            fresh: vec!["aaaaaaaa".into(), "aaaaaaaa".into(), "bbbbbbbb".into()],
            being_filed: 0,
        };
        let text = phrase(Path::new("/r/.git/greenharness/inbox.md"), 10, &scan);
        assert!(text.starts_with("greenbelay/inbox: /r/.git/greenharness/inbox.md holds 10 unfiled lines (nothing was moved):"), "{text}");
        assert!(text.contains("…\n"), "{text}");
        assert!(text.contains("- … and 2 more"), "{text}");
        assert!(text.contains("any session may file an orphan as a task ticket"));
        assert!(text.contains("3 lines under 2 days old, of sessions aaaaaaaa, bbbbbbbb:"));
    }

    #[test]
    fn only_a_fresh_start_is_told() {
        let dir = scratch("source");
        for source in ["resume", "compact", "fork", ""] {
            assert!(notice(&dir, source, Stance::Advise, "2026-11-17").is_none());
        }
    }
}
