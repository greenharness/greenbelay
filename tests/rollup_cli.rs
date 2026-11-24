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
