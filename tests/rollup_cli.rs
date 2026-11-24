//! `greenbelay rollup` through the binary: its flags, and what it refuses.

use std::path::PathBuf;
use std::process::Command;

struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Home {
        let dir =
            std::env::temp_dir().join(format!("greenbelay-ru-it-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // a fresh scratch dir: absent is the usual case
        std::fs::create_dir_all(dir.join("projects/p")).expect("projects dir");
        std::fs::create_dir_all(dir.join("greenbelay")).expect("state dir");
        Home(dir)
    }
    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_greenbelay"))
            .arg("rollup")
            .args(args)
            .env("CLAUDE_CONFIG_DIR", &self.0)
            .env_remove("XDG_STATE_HOME")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GREENBELAY_OFF")
            .output()
            .expect("the binary runs");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0); // Drop cannot report; a leftover temp dir is harmless
    }
}

#[test]
fn help_prints_usage_and_exits_0() {
    let home = Home::new("help");
    let (code, out, _) = home.run(&["--help"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: greenbelay rollup"), "{out}");
}

#[test]
fn weeks_outside_one_to_104_and_unknown_flags_exit_2() {
    let home = Home::new("flags");
    for args in [
        &["--weeks", "0"][..],
        &["--weeks=105"],
        &["--weeks"],
        &["--bogus"],
        &["--since", "2026-11-01"],
    ] {
        let (code, out, err) = home.run(args);
        assert_eq!(code, 2, "{args:?}: {out} {err}");
        assert!(out.is_empty(), "{args:?} printed to stdout: {out}");
    }
}

#[test]
fn weeks_takes_an_inline_value_and_a_transcripts_dir() {
    let home = Home::new("inline");
    let dir = home.0.display().to_string();
    let (code, out, err) = home.run(&["--weeks=3", "--json", "--transcripts", &dir]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("\"format\":\"greenbelay-rollup-v1\""), "{out}");
}

#[test]
fn transcripts_takes_an_inline_value() {
    let home = Home::new("inline-transcripts");
    let inline = format!("--transcripts={}", home.0.display());
    let (code, out, err) = home.run(&["--json", &inline]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("\"format\":\"greenbelay-rollup-v1\""), "{out}");
    let (code, _, _) = home.run(&["--transcripts="]);
    assert_eq!(code, 2);
}

#[test]
fn an_unknown_rollup_version_exits_1_with_no_report() {
    let home = Home::new("version");
    std::fs::write(
        home.0.join("greenbelay/rollup.log"),
        "greenbelay-rollup-v99\n",
    )
    .expect("a roll-up of another version");
    let dir = home.0.display().to_string();
    let (code, out, err) = home.run(&["--json", "--transcripts", &dir]);
    assert_eq!(code, 1, "{out} {err}");
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("format"), "{err}");
}

/// Proleptic Gregorian date of a day count since 1970-01-01 (Hinnant).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(unix)]
#[test]
fn an_unreadable_transcript_skips_the_freeze() {
    use std::os::unix::fs::PermissionsExt as _;
    let home = Home::new("unreadable");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is past the epoch")
        .as_secs();
    // A record a week old puts last week, a closed week, in reach of the freeze.
    let week_old = now - 7 * 86_400;
    std::fs::write(
        home.0.join("greenbelay/journal.log"),
        format!("F {week_old} r deny denied sess repo default x\n"),
    )
    .expect("a journal with one old record");
    let (year, month, day) = civil_from_days(week_old / 86_400);
    let entry = format!(
        "{{\"timestamp\":\"{year:04}-{month:02}-{day:02}T12:00:00Z\",\"sessionId\":\"s\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"name\":\"Bash\",\"id\":\"t1\",\"input\":{{\"command\":\"ls\"}}}}]}}}}\n"
    );
    std::fs::write(home.0.join("projects/p/readable.jsonl"), entry)
        .expect("a transcript that reads whole, with one call in the closed week");
    let transcript = home.0.join("projects/p/session.jsonl");
    std::fs::write(&transcript, "{}\n").expect("a transcript");
    std::fs::set_permissions(&transcript, std::fs::Permissions::from_mode(0o000))
        .expect("the transcript is made unreadable");
    if std::fs::File::open(&transcript).is_ok() {
        eprintln!(
            "skipped: this user ignores file modes, so the transcript cannot be made unreadable"
        );
        return;
    }
    let dir = home.0.display().to_string();
    let (code, out, err) = home.run(&["--json", "--transcripts", &dir]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(
        err.contains("transcript file(s) unreadable; calls of closed weeks not stored"),
        "{err}"
    );
    assert!(
        !home.0.join("greenbelay/rollup.log").exists(),
        "the freeze wrote a roll-up from a partial scan"
    );
}

#[test]
fn a_readable_transcript_freezes_the_closed_week() {
    let home = Home::new("freezes");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is past the epoch")
        .as_secs();
    let week_old = now - 7 * 86_400;
    std::fs::write(
        home.0.join("greenbelay/journal.log"),
        format!("F {week_old} r deny denied sess repo default x\n"),
    )
    .expect("a journal with one old record");
    let (year, month, day) = civil_from_days(week_old / 86_400);
    let entry = format!(
        "{{\"timestamp\":\"{year:04}-{month:02}-{day:02}T12:00:00Z\",\"sessionId\":\"s\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"name\":\"Bash\",\"id\":\"t1\",\"input\":{{\"command\":\"ls\"}}}}]}}}}\n"
    );
    std::fs::write(home.0.join("projects/p/readable.jsonl"), entry).expect("a transcript");
    let dir = home.0.display().to_string();
    let (code, out, err) = home.run(&["--json", "--transcripts", &dir]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(home.0.join("greenbelay/rollup.log").exists(), "{err}");
}
