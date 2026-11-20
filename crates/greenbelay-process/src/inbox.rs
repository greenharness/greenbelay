//! The unfiled inbox lines a session opening at `startup` or `clear` is told
//! about (read contract `inbox-read/1`, docs/index.md).
//!
//! A session that files its inbox lines at its boundary leaves nothing here;
//! a session that was cleared first, or that died, leaves lines behind. They
//! live in `<git-common-dir>/greenharness/inbox.md`, and a session that is
//! filing them moves the file to `inbox.<id>.filing` first, so an abandoned
//! take sits beside the inbox. Nothing is moved or written by reading.
//!
//! The line grammar is `<YYYY-MM-DD> <session8> [due=YYYY-MM-DD] <text>`.
//! The caller decides whether the session start is one to tell, finds the
//! inbox and reads the clock; this module only reads the files it is given.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use greenbelay_guard::civil::Day;
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
    /// Lines dated `ORPHAN_AGE_DAYS` or more before today, and every line of
    /// an abandoned take: no live session will file them.
    pub orphans: usize,
    /// Younger lines, which their own session files at its boundary.
    pub fresh: usize,
    /// Lines of a take a live session is filing now: not unfiled.
    pub being_filed: usize,
}

impl Notice {
    /// Whether a session can still file something here: orphans or fresh
    /// lines. A take being filed, or a notice that only counts, is not.
    pub fn has_unfiled(&self) -> bool {
        self.orphans + self.fresh > 0
    }
}

/// Notices shown at most, from the checkouts under a directory that is not
/// itself a repository.
const MAX_CHILD_NOTICES: usize = 3;

/// The text for a session started outside any repository. `children` are
/// `(name, quoted path, notice)` of the checkouts that hold an inbox. In name
/// order, the first three with unfiled lines and a text each follow an
/// `in <name>: cd <path>` line; the rest are counted in one closing line.
/// Notices that only file, or carry no text, take no slot and are not counted.
pub fn outside_notice(children: &[(String, String, Notice)]) -> Option<String> {
    let mut ordered: Vec<&(String, String, Notice)> = children
        .iter()
        .filter(|(_, _, found)| found.has_unfiled() && found.text.is_some())
        .collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    let mut shown: Vec<String> = ordered
        .iter()
        .take(MAX_CHILD_NOTICES)
        .filter_map(|(name, quoted, found)| {
            let text = found.text.as_deref()?;
            Some(format!("in {name}: cd {quoted}\n{text}"))
        })
        .collect();
    if shown.is_empty() {
        return None;
    }
    let unshown: Vec<&str> = ordered
        .iter()
        .skip(MAX_CHILD_NOTICES)
        .map(|(name, _, _)| name.as_str())
        .collect();
    if !unshown.is_empty() {
        let mut names: Vec<&str> = unshown.iter().take(MAX_CHILD_NOTICES).copied().collect();
        if unshown.len() > MAX_CHILD_NOTICES {
            names.push("…");
        }
        let (count, verb, noun) = if unshown.len() == 1 {
            (1, "holds", "repository")
        } else {
            (unshown.len(), "hold", "repositories")
        };
        shown.push(format!(
            concat!(
                greenbelay_guard::bin!(),
                "/inbox: {} more {} {} unfiled lines: {}; start a session in one to see them"
            ),
            count,
            noun,
            verb,
            names.join(", ")
        ));
    }
    Some(shown.join("\n"))
}

/// The inbox notice for `inbox`, or `None` when nothing scanned has a line.
/// `today` is the local calendar day, the clock the writers date lines by;
/// `now` ages the takes.
pub fn notice(inbox: &Path, stance: Stance, today: Day, now: SystemTime) -> Option<Notice> {
    let scan = scan(inbox, today, now);
    if scan.orphans.is_empty() && scan.fresh.is_empty() && scan.being_filed == 0 {
        return None;
    }
    // A take being filed alone asks nothing of the reader: it is counted and
    // journalled, never said.
    let unfiled = !scan.orphans.is_empty() || !scan.fresh.is_empty();
    let text = match stance {
        Stance::Advise | Stance::Deny if unfiled => Some(phrase(inbox, &scan)),
        _ => None,
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

fn scan(inbox: &Path, today: Day, now: SystemTime) -> Scan {
    let mut scan = Scan::default();
    // The takes are listed before the inbox is read: a session that moves the
    // inbox to a take in between is then missed once, never counted twice.
    let takes = takes_beside(inbox);
    if let Some(text) = read(inbox) {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match session_of_fresh(line, today) {
                Some(session) => scan.fresh.push(session),
                None => scan.orphans.push(line.to_string()),
            }
        }
    }
    for take in takes {
        // A take that vanished or cannot be dated is skipped: its filer is
        // still at work, or done.
        let Some(age) = take_age(&take, now) else {
            continue;
        };
        let Some(text) = read(&take) else { continue };
        let lines = text.lines().filter(|l| !l.trim().is_empty());
        if age > TAKE_ABANDONED_AFTER {
            scan.orphans.extend(lines.map(str::to_string));
        } else {
            scan.being_filed += lines.count();
        }
    }
    scan
}

fn takes_beside(inbox: &Path) -> Vec<PathBuf> {
    let Some(dir) = inbox.parent() else {
        return Vec::new();
    };
    // No directory means no inbox and no take: nothing to tell.
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
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
    takes
}

/// How long ago a take was made: its modification time, which the filer
/// sets with `touch` right after the move (contract `inbox-read/1`), so a
/// take is aged from the move and not from the inbox's last append.
fn take_age(take: &Path, now: SystemTime) -> Option<Duration> {
    let made = std::fs::metadata(take).ok()?.modified().ok()?;
    Some(now.duration_since(made).unwrap_or_default())
}

/// The file's lines, a bad byte spoiling only its own line. A missing or
/// unreadable file is no lines: the plan keeps read errors silent.
fn read(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// The session id of a line that is not an orphan: dated under
/// `ORPHAN_AGE_DAYS` ago, or malformed (session `?`). `None` is an orphan.
fn session_of_fresh(line: &str, today: Day) -> Option<String> {
    let mut words = line.split_whitespace();
    let date = words
        .next()
        .filter(|w| w.len() == 10)
        .and_then(greenbelay_guard::civil::day_of);
    let session = words.next();
    match (date, session) {
        (Some(day), Some(session)) => (today - day < ORPHAN_AGE_DAYS).then(|| session.to_string()),
        _ => Some(UNKNOWN_SESSION.to_string()),
    }
}

/// Where an unfiled line goes, written into the orphan block and the
/// fresh block alike, so either block stands alone.
const WHERE_TO_FILE: &str = "as a task ticket in the plan's programme map, \
     or else in docs/programmes/follow-ups/, then remove its line";

fn phrase(inbox: &Path, scan: &Scan) -> String {
    let unfiled = scan.orphans.len() + scan.fresh.len();
    let mut out = format!(
        concat!(
            greenbelay_guard::bin!(),
            "/inbox: {} holds {} unfiled (nothing was moved):"
        ),
        inbox.display(),
        lines_of(unfiled)
    );
    if !scan.orphans.is_empty() {
        out.push_str(&format!(
            "\n{} orphaned (dated {ORPHAN_AGE_DAYS} days ago or more, or in an abandoned take):",
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
        out.push_str("\nany session may file an orphan: ");
        out.push_str(WHERE_TO_FILE);
    }
    if scan.being_filed > 0 {
        out.push_str(&format!(
            "\n{} being filed by a live session (not shown)",
            lines_of(scan.being_filed)
        ));
    }
    if !scan.fresh.is_empty() {
        let mut sessions: Vec<&str> = Vec::new();
        for session in &scan.fresh {
            if !sessions.contains(&session.as_str()) {
                sessions.push(session);
            }
        }
        out.push_str(&format!(
            "\n{} under {ORPHAN_AGE_DAYS} days old, by session id: {}\n\
             a live session files its own at its boundary; after a /clear, \
             the lines of the cleared session's id are yours to file: {WHERE_TO_FILE}",
            lines_of(scan.fresh.len()),
            sessions.join(", "),
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
    use greenbelay_guard::civil;

    fn child(
        name: &str,
        orphans: usize,
        fresh: usize,
        being_filed: usize,
    ) -> (String, String, Notice) {
        (
            name.to_string(),
            format!("'/w/{name}'"),
            Notice {
                text: Some(format!("text-{name}")),
                orphans,
                fresh,
                being_filed,
            },
        )
    }

    #[test]
    fn seven_children_show_three_and_count_four_with_an_ellipsis() {
        let children: Vec<_> = ["g", "a", "f", "b", "e", "c", "d"]
            .iter()
            .map(|n| child(n, 1, 0, 0))
            .collect();
        let out = outside_notice(&children).unwrap();
        assert_eq!(
            out,
            concat!(
                "in a: cd '/w/a'\ntext-a\n",
                "in b: cd '/w/b'\ntext-b\n",
                "in c: cd '/w/c'\ntext-c\n",
                "greenbelay/inbox: 4 more repositories hold unfiled lines: d, e, f, …; ",
                "start a session in one to see them"
            )
        );
    }

    #[test]
    fn one_more_repository_uses_the_singular() {
        let children: Vec<_> = ["a", "b", "c", "d"]
            .iter()
            .map(|n| child(n, 0, 2, 0))
            .collect();
        let out = outside_notice(&children).unwrap();
        assert!(out.ends_with(
            "greenbelay/inbox: 1 more repository holds unfiled lines: d; start a session in one to see them"
        ));
    }

    #[test]
    fn filing_only_children_take_no_slot_and_are_not_counted() {
        let children = vec![
            child("a", 0, 0, 3),
            child("b", 1, 0, 0),
            child("c", 0, 0, 1),
        ];
        assert_eq!(
            outside_notice(&children).unwrap(),
            "in b: cd '/w/b'\ntext-b"
        );
        assert_eq!(outside_notice(&[child("a", 0, 0, 3)]), None);
    }

    fn today() -> Day {
        civil::day_of("2026-11-17").unwrap()
    }

    /// A fresh directory for one test, removed again by [`Scratch::drop`].
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("greenbelay-inbox-{name}-{}", std::process::id()));
            if dir.exists() {
                std::fs::remove_dir_all(&dir).unwrap();
            }
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn inbox(&self) -> PathBuf {
            self.0.join("inbox.md")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_dir_all(&self.0) {
                eprintln!("could not remove {}: {error}", self.0.display());
            }
        }
    }

    fn orphan(line: &str) -> bool {
        session_of_fresh(line, today()).is_none()
    }

    fn a_day_later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(25 * 60 * 60)
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
        let scratch = Scratch::new("takes");
        let take = scratch.0.join("inbox.abc.filing");
        std::fs::write(&take, "2026-11-17 abcd1234 a\n2026-11-17 abcd1234 b\n").unwrap();
        let young = scan(&scratch.inbox(), today(), SystemTime::now());
        assert_eq!((young.orphans.len(), young.being_filed), (0, 2));
        let old = scan(&scratch.inbox(), today(), a_day_later());
        assert_eq!((old.orphans.len(), old.being_filed), (2, 0));
    }

    #[test]
    fn a_take_is_read_beside_an_empty_or_missing_inbox() {
        let scratch = Scratch::new("empty");
        std::fs::write(scratch.0.join("inbox.x.filing"), "2026-11-10 abcd1234 a\n").unwrap();
        assert_eq!(
            scan(&scratch.inbox(), today(), a_day_later()).orphans.len(),
            1
        );
        std::fs::write(scratch.inbox(), "").unwrap();
        assert_eq!(
            scan(&scratch.inbox(), today(), a_day_later()).orphans.len(),
            1
        );
    }

    #[test]
    fn a_bad_byte_spoils_only_its_own_line() {
        let scratch = Scratch::new("bytes");
        std::fs::write(
            scratch.inbox(),
            b"2026-11-10 abcd1234 caf\xff\n2026-11-10 abcd1234 fine\n",
        )
        .unwrap();
        assert_eq!(
            scan(&scratch.inbox(), today(), SystemTime::now())
                .orphans
                .len(),
            2
        );
    }

    #[test]
    fn the_stance_decides_whether_it_speaks() {
        let scratch = Scratch::new("stance");
        std::fs::write(scratch.inbox(), "2026-11-10 abcd1234 a\n").unwrap();
        let now = SystemTime::now();
        let said = notice(&scratch.inbox(), Stance::Advise, today(), now).unwrap();
        assert!(said.text.is_some());
        assert_eq!((said.orphans, said.fresh, said.being_filed), (1, 0, 0));
        let watched = notice(&scratch.inbox(), Stance::Observe, today(), now).unwrap();
        assert!(watched.text.is_none());
        assert_eq!(watched.orphans, 1);
    }

    #[test]
    fn a_young_take_alone_is_counted_but_not_said() {
        let scratch = Scratch::new("young-take");
        std::fs::write(
            scratch.0.join("inbox.abcd1234.1.filing"),
            "2026-11-17 abcd1234 a\n2026-11-17 abcd1234 b\n",
        )
        .unwrap();
        let found = notice(&scratch.inbox(), Stance::Advise, today(), SystemTime::now()).unwrap();
        assert_eq!(found.text, None);
        assert_eq!((found.orphans, found.fresh, found.being_filed), (0, 0, 2));
        std::fs::write(scratch.inbox(), "2026-11-17 feed0001 c\n").unwrap();
        let with_fresh =
            notice(&scratch.inbox(), Stance::Advise, today(), SystemTime::now()).unwrap();
        let text = with_fresh.text.unwrap();
        assert!(text.contains("holds 1 line unfiled"), "{text}");
        assert!(
            text.contains("2 lines being filed by a live session"),
            "{text}"
        );
    }

    #[test]
    fn nothing_scanned_is_no_notice() {
        let scratch = Scratch::new("nothing");
        assert!(notice(&scratch.inbox(), Stance::Advise, today(), SystemTime::now()).is_none());
    }

    #[test]
    fn the_text_cuts_by_chars_and_counts_only_unfiled_lines() {
        let long = format!("2026-11-01 abcd1234 {}", "é".repeat(300));
        let scan = Scan {
            orphans: vec![long; 7],
            fresh: vec!["aaaaaaaa".into(), "aaaaaaaa".into(), "bbbbbbbb".into()],
            being_filed: 4,
        };
        let text = phrase(Path::new("/r/.git/greenharness/inbox.md"), &scan);
        assert!(
            text.starts_with(
                "greenbelay/inbox: /r/.git/greenharness/inbox.md holds 10 lines unfiled (nothing was moved):"
            ),
            "{text}"
        );
        assert!(text.contains("…\n"), "{text}");
        assert!(text.contains("- … and 2 more"), "{text}");
        assert!(text.contains("any session may file an orphan"), "{text}");
        assert!(
            text.contains("4 lines being filed by a live session"),
            "{text}"
        );
        assert!(
            text.contains("3 lines under 2 days old, by session id: aaaaaaaa, bbbbbbbb\n"),
            "{text}"
        );
    }

    #[test]
    fn fresh_lines_alone_say_where_to_file() {
        let scan = Scan {
            orphans: Vec::new(),
            fresh: vec!["aaaaaaaa".into()],
            being_filed: 0,
        };
        let text = phrase(Path::new("/r/.git/greenharness/inbox.md"), &scan);
        assert!(!text.contains("orphan"), "{text}");
        assert!(
            text.ends_with(
                "the lines of the cleared session's id are yours to file: \
                 as a task ticket in the plan's programme map, \
                 or else in docs/programmes/follow-ups/, then remove its line"
            ),
            "{text}"
        );
    }
}
