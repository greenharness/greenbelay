//! `greenbelay owner claim|release|show`, with a sandboxed config dir and host
//! registry: nothing here reads or writes the real `~/.claude`.

use std::path::PathBuf;
use std::process::{Command, Output};

const ALPHA_PID: u32 = 41_001;
const BETA_PID: u32 = 41_002;
const SESSION_A: &str = "aaaaaaaa-1111-4000-8000-000000000001";
const SESSION_B: &str = "bbbbbbbb-1111-4000-8000-000000000002";

struct Scene {
    root: PathBuf,
    feature: PathBuf,
}

impl Scene {
    fn new(name: &str) -> Scene {
        let root = std::env::temp_dir().join(format!("gb-owner-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // a leftover of an earlier run of this test
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let primary = root.join("repo");
        std::fs::create_dir_all(primary.join(".git/worktrees/feat")).unwrap();
        std::fs::write(primary.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            primary.join(".git/worktrees/feat/HEAD"),
            "ref: refs/heads/feat/x\n",
        )
        .unwrap();
        std::fs::write(primary.join(".git/worktrees/feat/commondir"), "../..\n").unwrap();
        let feature = root.join("feat");
        std::fs::create_dir_all(&feature).unwrap();
        std::fs::write(
            feature.join(".git"),
            format!(
                "gitdir: {}\n",
                primary.join(".git/worktrees/feat").display()
            ),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        for (pid, session, name) in [
            (ALPHA_PID, SESSION_A, "alpha-name"),
            (BETA_PID, SESSION_B, "beta-name"),
        ] {
            std::fs::write(
                root.join("sessions").join(format!("{pid}.json")),
                format!(
                    r#"{{"sessionId":"{session}","name":"{name}","pid":{pid},"startedAt":1700000000000,"peerProtocol":1}}"#
                ),
            )
            .unwrap();
        }
        Scene { root, feature }
    }

    fn owner(&self, caller_pid: u32, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_greenbelay"))
            .arg("owner")
            .args(args)
            .current_dir(&self.feature)
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("config"))
            .env("GREENBELAY_HOST_SESSIONS_DIR", self.root.join("sessions"))
            .env("GREENBELAY_TEST_CALLER_PID", caller_pid.to_string())
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("XDG_STATE_HOME")
            .env_remove("GREENBELAY_OFF")
            .output()
            .expect("the binary runs")
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root); // scratch only
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_second_claim_is_refused_with_the_holder_and_the_next_command() {
    let scene = Scene::new("refused");
    let first = scene.owner(ALPHA_PID, &["claim", "--ref", "417dc6"]);
    assert_eq!(first.status.code(), Some(0), "{}", text(&first.stderr));

    let second = scene.owner(BETA_PID, &["claim"]);
    assert_eq!(second.status.code(), Some(3));
    let said = text(&second.stderr);
    for part in [
        "refused: alpha-name [417dc6] holds worktree ",
        ", live. Message it: SendMessage to \"alpha-name\". To take over anyway: greenbelay owner claim --steal",
    ] {
        assert!(said.contains(part), "{part}: {said}");
    }
    assert_eq!(text(&second.stdout), "");

    let ticket = scene.owner(ALPHA_PID, &["claim", "--ticket", "repo/follow-ups/t02"]);
    assert_eq!(ticket.status.code(), Some(0), "{}", text(&ticket.stderr));
    let refused = scene.owner(
        BETA_PID,
        &["claim", "--ticket", "repo/follow-ups/t02", "--steal"],
    );
    assert_eq!(refused.status.code(), Some(0), "{}", text(&refused.stderr));
    assert!(text(&refused.stdout).is_empty(), "done is said on stderr");

    let stolen = scene.owner(BETA_PID, &["claim", "--steal"]);
    assert_eq!(stolen.status.code(), Some(0), "{}", text(&stolen.stderr));
}

#[test]
fn an_unreadable_record_holds_until_a_steal_and_a_gone_owner_is_free() {
    let scene = Scene::new("unreadable");
    let first = scene.owner(ALPHA_PID, &["claim", "--ticket", "repo/follow-ups/t09"]);
    assert_eq!(first.status.code(), Some(0), "{}", text(&first.stderr));
    let record = scene
        .root
        .join("config/greenbelay/owners/ticket/repo/follow-ups/t09.json");
    std::fs::write(&record, "{\"v\":2}").unwrap();
    // A holds the worktree, so only the ticket's unreadable record refuses.
    let refused = scene.owner(ALPHA_PID, &["claim", "--ticket", "repo/follow-ups/t09"]);
    assert_eq!(refused.status.code(), Some(3));
    let said = text(&refused.stderr);
    assert!(said.contains("does not read"), "{said}");
    assert!(
        said.contains("greenbelay owner claim --ticket repo/follow-ups/t09 --steal"),
        "{said}"
    );
    assert_eq!(std::fs::read_to_string(&record).unwrap(), "{\"v\":2}");
    let shown = scene.owner(
        BETA_PID,
        &["show", "--ticket", "repo/follow-ups/t09", "--json"],
    );
    let value: serde_json::Value = serde_json::from_str(&text(&shown.stdout)).unwrap();
    assert_eq!(value["state"], "unknown");
    assert!(
        value["session"].is_null() && value.get("name").is_some(),
        "{value}"
    );
    let stolen = scene.owner(
        BETA_PID,
        &["claim", "--ticket", "repo/follow-ups/t09", "--steal"],
    );
    assert_eq!(stolen.status.code(), Some(0), "{}", text(&stolen.stderr));
}

#[test]
fn show_names_the_live_owner_and_release_frees_it() {
    let scene = Scene::new("show");
    scene.owner(ALPHA_PID, &["claim"]);

    let shown = scene.owner(BETA_PID, &["show", "--json"]);
    assert_eq!(shown.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_str(&text(&shown.stdout)).unwrap();
    assert_eq!(value["name"], "alpha-name");
    assert_eq!(value["session"], SESSION_A);
    assert_eq!(value["state"], "live");

    let refused = scene.owner(BETA_PID, &["release"]);
    assert_eq!(refused.status.code(), Some(3));

    let released = scene.owner(ALPHA_PID, &["release"]);
    assert_eq!(released.status.code(), Some(0));
    let again = scene.owner(ALPHA_PID, &["release"]);
    assert_eq!(again.status.code(), Some(0), "already free is success");

    let none = scene.owner(ALPHA_PID, &["show"]);
    assert_eq!(none.status.code(), Some(1));
    assert_eq!(text(&none.stdout), "");
    assert!(text(&none.stderr).starts_with("no owner recorded for "));
}

#[test]
fn a_ticket_is_claimed_and_shown_by_its_key() {
    let scene = Scene::new("ticket");
    let claimed = scene.owner(ALPHA_PID, &["claim", "--ticket", "repo/follow-ups/t02"]);
    assert_eq!(claimed.status.code(), Some(0), "{}", text(&claimed.stderr));
    let shown = scene.owner(
        BETA_PID,
        &["show", "--ticket", "repo/follow-ups/t02", "--json"],
    );
    let value: serde_json::Value = serde_json::from_str(&text(&shown.stdout)).unwrap();
    assert_eq!(value["name"], "alpha-name");
    assert_eq!(value["place"], "ticket");
}

#[test]
fn a_usage_error_exits_2_and_help_documents_the_codes() {
    let scene = Scene::new("usage");
    for args in [
        &["bogus"][..],
        &["claim", "--bogus"],
        &["claim", "--ticket", "not-a-key"],
        &["show", "--ticket", "a/b/c", "--worktree", "/x"],
    ] {
        assert_eq!(
            scene.owner(ALPHA_PID, args).status.code(),
            Some(2),
            "{args:?}"
        );
    }
    let help = scene.owner(ALPHA_PID, &["--help"]);
    assert_eq!(help.status.code(), Some(0));
    let said = text(&help.stdout);
    for part in ["exit status", "3  another session holds it"] {
        assert!(said.contains(part), "{part}: {said}");
    }
}
