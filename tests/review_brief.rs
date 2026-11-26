//! `amont-agent rules-sha` and `review-brief`, through the real binary
//! (decisions ADR-0043, ADR-0044).

use std::path::PathBuf;
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "greenbelay-review-brief-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("cannot clear {}: {e}", dir.display()),
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs the binary with the fake `greentopo` in `bin` first on PATH, so no test
/// reaches the machine's own `greentopo`.
fn run(args: &[&str], bin: &std::path::Path) -> Output {
    let mut cmd = greenbelay_command();
    cmd.args(args);
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![bin.to_path_buf()];
    dirs.extend(std::env::split_paths(&inherited));
    cmd.env("PATH", std::env::join_paths(dirs).unwrap());
    // No test reaches the machine's own `greenspotter`: a fake in `bin`, or none.
    cmd.env("GREENBELAY_GREENSPOTTER", bin.join("greenspotter"));
    cmd.output().expect("greenbelay runs")
}

/// The fake is a `#!/bin/sh` script: Windows resolves `Command::new("greentopo")`
/// to `greentopo.exe` only, so a `.cmd` fake would never be found. Every test that
/// needs it is therefore gated `cfg(unix)`, and none runs the real `greentopo`.
#[cfg(unix)]
fn fake_greentopo(dir: &std::path::Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("greentopo");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn a_failing_greentopo_exits_3_with_nothing_on_stdout() {
    let bin = scratch("failing");
    fake_greentopo(&bin, "#!/bin/sh\nexit 1\n");
    for sub in ["rules-sha", "review-brief"] {
        let out = run(&[sub, "-C", "."], &bin);
        assert_eq!(out.status.code(), Some(3), "{sub}");
        assert!(out.stdout.is_empty(), "{sub}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(err.trim().lines().count(), 1, "{err}");
        assert!(err.contains("greentopo --json rules"), "{err}");
    }
}

#[cfg(unix)]
#[test]
fn greentopo_rules_that_are_not_json_exit_3_with_nothing_on_stdout() {
    let bin = scratch("not-json");
    fake_greentopo(&bin, "#!/bin/sh\nprintf 'not json'\n");
    let out = run(&["review-brief", "-C", "."], &bin);
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn rules_sha_prints_the_sha256_of_what_greentopo_printed() {
    let bin = scratch("sha");
    fake_greentopo(&bin, "#!/bin/sh\nprintf abc\n");
    let out = run(&["rules-sha"], &bin);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n"
    );
}

#[cfg(unix)]
#[test]
fn both_are_listed_and_answer_help() {
    let bin = scratch("help");
    fake_greentopo(&bin, "#!/bin/sh\nexit 1\n");
    let help = run(&["--help"], &bin);
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("rules-sha"), "{text}");
    assert!(text.contains("review-brief"), "{text}");
    for sub in ["rules-sha", "review-brief"] {
        assert_eq!(run(&[sub, "--help"], &bin).status.code(), Some(0), "{sub}");
    }
}

#[cfg(unix)]
#[test]
fn a_usage_error_exits_2() {
    let bin = scratch("usage");
    fake_greentopo(&bin, "#!/bin/sh\nexit 1\n");
    for sub in ["rules-sha", "review-brief"] {
        assert_eq!(run(&[sub, "--bogus"], &bin).status.code(), Some(2), "{sub}");
        assert_eq!(run(&[sub, "-C"], &bin).status.code(), Some(2), "{sub}");
        // A word whose second byte is inside a character is a usage error, not a panic.
        assert_eq!(run(&[sub, "aé"], &bin).status.code(), Some(2), "{sub}");
    }
    assert_eq!(
        run(&["rules-sha", "--plan", "x"], &bin).status.code(),
        Some(2)
    );
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

#[cfg(unix)]
#[test]
fn a_failed_git_diff_is_a_repository_error_with_nothing_on_stdout() {
    let bin = scratch("diff-fails");
    fake_greentopo(&bin, "#!/bin/sh\nprintf '{\"rules\":[]}'\n");
    let repo = scratch("diff-fails-repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    git(&repo, &["commit", "-qam", "change"]);
    // Remove the changed file's new blob: commits and trees still read, a diff cannot.
    let blob = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "HEAD:a.txt"])
        .output()
        .unwrap();
    let blob = String::from_utf8(blob.stdout).unwrap();
    let blob = blob.trim();
    let object = repo.join(".git/objects").join(&blob[..2]).join(&blob[2..]);
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&object, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    std::fs::remove_file(&object).unwrap();

    let out = run(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
        ],
        &bin,
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("git diff"),
        "{out:?}"
    );
}

/// A scratch repository with a base commit and one change on top, for the
/// brief's success path and the flag shapes.
#[cfg(unix)]
fn two_commit_repo(name: &str) -> PathBuf {
    two_commit_repo_with(name, &[".adr.yaml"])
}

/// The same, with `extra` files in the base commit (`.adr.yaml` marks a
/// repository that has an greentopo corpus).
#[cfg(unix)]
fn two_commit_repo_with(name: &str, extra: &[&str]) -> PathBuf {
    let repo = scratch(name);
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    for file in extra {
        std::fs::write(repo.join(file), "x: 1\n").unwrap();
    }
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    // Two languages, so a language profile's brief has something to leave out.
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::create_dir_all(repo.join("web")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn one() {}\n").unwrap();
    std::fs::write(repo.join("web/app.ts"), "export const one = 1;\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn two() {}\n").unwrap();
    std::fs::write(repo.join("web/app.ts"), "export const two = 2;\n").unwrap();
    git(&repo, &["commit", "-qam", "change"]);
    repo
}

/// An `greentopo` that answers each command the brief runs with fixed text.
#[cfg(unix)]
const BRIEF_GREENTOPO: &str = r#"#!/bin/sh
case "$*" in
  "--json rules --level constraint --governs code --full")
    printf '{"rules":[{"id":"names.reveal-intent","level":"constraint","statement":"A name says what it is.","body":"No placeholder names.","active":true}]}' ;;
  "rules --level constraint") printf 'constraint names.reveal-intent\n' ;;
  "--json relevant"*) printf '{"keys":[]}' ;;
  *) exit 1 ;;
esac
"#;

#[cfg(unix)]
#[test]
fn the_brief_binds_the_same_tree_and_rules_as_tree_sha_and_rules_sha() {
    let bin = scratch("brief-ok");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let repo = two_commit_repo("brief-ok-repo");
    let dir = repo.to_str().unwrap();
    let brief = run(&["review-brief", "-C", dir, "--base", "HEAD~1"], &bin);
    assert_eq!(brief.status.code(), Some(0), "{brief:?}");
    let brief = String::from_utf8(brief.stdout).unwrap();
    let rules_sha = run(&["rules-sha", "-C", dir], &bin);
    assert_eq!(rules_sha.status.code(), Some(0), "{rules_sha:?}");
    let rules_sha = String::from_utf8(rules_sha.stdout).unwrap();
    assert_eq!(rules_sha.trim().len(), 64, "{rules_sha}");
    let tree = run(&["tree-sha", "--block", "-C", dir], &bin);
    assert_eq!(tree.status.code(), Some(0), "{tree:?}");
    let tree = String::from_utf8(tree.stdout).unwrap();
    assert!(tree.trim().starts_with("<<<TREE "), "{tree}");
    assert!(
        brief.contains(&format!("<<<RULES sha={}>>>", rules_sha.trim())),
        "{brief}"
    );
    assert!(brief.contains(tree.trim()), "{brief}");
    assert!(brief.contains("No placeholder names."), "{brief}");
}

#[cfg(unix)]
#[test]
fn flags_take_the_posix_shapes() {
    let bin = scratch("posix");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let repo = two_commit_repo("posix-repo");
    let attached = format!("-C{}", repo.display());
    for args in [
        vec!["review-brief", attached.as_str(), "--base=HEAD~1"],
        vec!["review-brief", attached.as_str(), "--base=HEAD~1", "--"],
        vec!["rules-sha", attached.as_str()],
    ] {
        let out = run(&args, &bin);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {out:?}");
    }
    let operand = run(&["rules-sha", attached.as_str(), "--", "extra"], &bin);
    assert_eq!(operand.status.code(), Some(2), "{operand:?}");
}

#[cfg(unix)]
#[test]
fn an_greentopo_relevant_answer_without_keys_exits_3() {
    let bin = scratch("no-keys");
    fake_greentopo(
        &bin,
        &BRIEF_GREENTOPO.replace(
            r#""--json relevant"*) printf '{"keys":[]}' ;;"#,
            r#""--json relevant"*) printf '{}' ;;"#,
        ),
    );
    let repo = two_commit_repo("no-keys-repo");
    let out = run(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
        ],
        &bin,
    );
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("keys"),
        "{out:?}"
    );
}

const ID_ONE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const ID_TWO: &str = "2222222222222222222222222222222222222222222222222222222222222222";

/// A fake `greenspotter` that records its arguments and prints `document`.
#[cfg(unix)]
fn fake_greenspotter(dir: &std::path::Path, document: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(dir.join("candidates.json"), document).unwrap();
    let path = dir.join("greenspotter");
    std::fs::write(
        &path,
        "#!/bin/sh\nd=$(dirname \"$0\")\nprintf '%s' \"$*\" > \"$d/args\"\ncat \"$d/candidates.json\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn two_candidates() -> String {
    let row = |id: &str, line: u32| {
        format!(
            r#"{{"id":"{id}","rule":"names.reveal-intent","pattern_id":"placeholder","pattern":"a placeholder name","path":"src/a.rs","line":{line},"col":5,"text":"let data = 1;","future":true}}"#
        )
    };
    format!(
        r#"{{"format":"greenspotter.candidates/1","base":"b","head":"h","candidates":[{},{}],"overflow":{{"names.reveal-intent":7}}}}"#,
        row(ID_ONE, 3),
        row(ID_TWO, 9)
    )
}

#[cfg(unix)]
fn brief_text(bin: &std::path::Path, repo: &std::path::Path) -> (Output, String) {
    let out = run(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
        ],
        bin,
    );
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    (out, text)
}

#[cfg(unix)]
#[test]
fn the_brief_carries_greenspotters_candidates_right_after_the_rules_line() {
    let bin = scratch("cand-two");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let document = two_candidates();
    fake_greenspotter(&bin, &document);
    let repo = two_commit_repo("cand-two-repo");
    let (out, brief) = brief_text(&bin, &repo);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    let sha = greenbelay_guard::sha256::hex(&greenbelay_guard::sha256::sha256(document.as_bytes()));
    let lines: Vec<&str> = brief.lines().collect();
    let rules = lines
        .iter()
        .position(|l| l.starts_with("<<<RULES sha="))
        .expect("a rules line");
    assert_eq!(lines[rules + 1], format!("<<<CANDIDATES sha={sha} n=2>>>"));
    assert_eq!(
        lines[rules + 2],
        format!("{ID_ONE} names.reveal-intent placeholder src/a.rs:3")
    );
    assert_eq!(
        lines[rules + 3],
        format!("{ID_TWO} names.reveal-intent placeholder src/a.rs:9")
    );
    assert_eq!(lines[rules + 4], "<<<END CANDIDATES>>>");
    assert!(brief.contains("## Candidates"), "{brief}");
    assert!(brief.contains("a placeholder name"), "{brief}");
    assert!(brief.contains("let data = 1;"), "{brief}");
    assert!(brief.contains("names.reveal-intent: 7"), "{brief}");
    assert!(brief.contains("`Candidates:` block"), "{brief}");
    let base = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "HEAD~1"])
        .output()
        .unwrap();
    let base = String::from_utf8(base.stdout).unwrap();
    assert_eq!(
        std::fs::read_to_string(bin.join("args")).unwrap(),
        format!("candidates --base {} --json --max 40", base.trim())
    );
}

#[cfg(unix)]
#[test]
fn no_candidates_is_an_empty_block_and_no_ruling_is_needed() {
    let bin = scratch("cand-zero");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let document = r#"{"format":"greenspotter.candidates/1","base":"b","head":"h","candidates":[],"overflow":{}}"#;
    fake_greenspotter(&bin, document);
    let repo = two_commit_repo("cand-zero-repo");
    let (out, brief) = brief_text(&bin, &repo);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let sha = greenbelay_guard::sha256::hex(&greenbelay_guard::sha256::sha256(document.as_bytes()));
    assert!(
        brief.contains(&format!(
            "<<<CANDIDATES sha={sha} n=0>>>\n<<<END CANDIDATES>>>\n"
        )),
        "{brief}"
    );
    assert!(brief.contains("no ruling is needed"), "{brief}");
}

#[cfg(unix)]
#[test]
fn a_missing_greenspotter_leaves_the_brief_with_an_unavailable_marker() {
    let bin = scratch("cand-missing");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let repo = two_commit_repo("cand-missing-repo");
    let (out, brief) = brief_text(&bin, &repo);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(brief.contains("<<<CANDIDATES unavailable>>>"), "{brief}");
    assert!(brief.contains("## Candidates"), "{brief}");
    assert!(brief.contains("could not run"), "{brief}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(err.trim().lines().count(), 1, "{err}");
    assert!(err.contains("greenspotter"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_wrong_format_token_or_a_failing_greenspotter_is_unavailable_too() {
    let bin = scratch("cand-wrong");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    fake_greenspotter(
        &bin,
        r#"{"format":"greenspotter.candidates/2","candidates":[]}"#,
    );
    let repo = two_commit_repo("cand-wrong-repo");
    let (out, brief) = brief_text(&bin, &repo);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(brief.contains("<<<CANDIDATES unavailable>>>"), "{brief}");
    assert!(brief.contains("candidates/2"), "{brief}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("candidates/2"),
        "{out:?}"
    );

    use std::os::unix::fs::PermissionsExt;
    let failing = bin.join("greenspotter");
    std::fs::write(&failing, "#!/bin/sh\necho broken >&2\nexit 4\n").unwrap();
    std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (out, brief) = brief_text(&bin, &repo);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(brief.contains("<<<CANDIDATES unavailable>>>"), "{brief}");
    assert!(brief.contains("broken"), "{brief}");
}

#[cfg(unix)]
#[test]
fn a_repository_without_a_corpus_gets_a_brief_without_rules() {
    let bin = scratch("no-corpus");
    fake_greentopo(&bin, "#!/bin/sh\nexit 1\n");
    fake_greenspotter(
        &bin,
        r#"{"format":"greenspotter.candidates/1","base":"b","head":"h","candidates":[],"overflow":{}}"#,
    );
    let repo = two_commit_repo_with("no-corpus-repo", &[]);
    let (out, brief) = brief_text(&bin, &repo);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!brief.contains("<<<RULES"), "{brief}");
    assert!(brief.contains("# Code rules"), "{brief}");
    assert!(brief.contains("no greentopo corpus"), "{brief}");
    assert!(!brief.contains("# Constraint rules"), "{brief}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(err.trim().lines().count(), 1, "{err}");
    assert!(err.contains(".adr.yaml"), "{err}");
    let sha = run(&["rules-sha", "-C", repo.to_str().unwrap()], &bin);
    assert_eq!(sha.status.code(), Some(3), "{sha:?}");
}

#[cfg(unix)]
#[test]
fn an_greentopo_keys_row_without_a_string_key_is_an_error_naming_the_row() {
    let bin = scratch("keyless-row");
    fake_greentopo(
        &bin,
        &BRIEF_GREENTOPO.replace(
            r#""--json relevant"*) printf '{"keys":[]}' ;;"#,
            r#""--json relevant"*) printf '{"keys":[{"key":"a.b"},{"score":1}]}' ;;"#,
        ),
    );
    let repo = two_commit_repo("keyless-row-repo");
    let (out, _) = brief_text(&bin, &repo);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("row 1") && err.contains("key"), "{err}");
}

/// Runs the binary with a PATH that holds only `dir` and a directory with a
/// symlink to `git`, so the machine's own `greentopo` is never found.
#[cfg(unix)]
fn run_without_greentopo(args: &[&str], dir: &std::path::Path, greentopo: Option<&str>) -> Output {
    let git_path = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let only = dir.join("only-git");
    std::fs::create_dir_all(&only).unwrap();
    let link = only.join("git");
    if !link.exists() {
        std::os::unix::fs::symlink(git_path.trim(), &link).unwrap();
    }
    let mut cmd = greenbelay_command();
    cmd.args(args);
    cmd.env("PATH", std::env::join_paths([dir, &only]).unwrap());
    cmd.env("GREENBELAY_GREENSPOTTER", dir.join("greenspotter"));
    cmd.env_remove("GREENBELAY_GREENTOPO");
    if let Some(program) = greentopo {
        cmd.env("GREENBELAY_GREENTOPO", program);
    }
    cmd.output().expect("greenbelay runs")
}

#[cfg(unix)]
#[test]
fn without_greentopo_the_brief_says_the_rules_were_not_read_and_exits_0() {
    // No greenspotter either: its candidates are unavailable at once, so no stub
    // can outlast the 30 s deadline on a loaded machine.
    let bin = scratch("no-greentopo");
    let repo = two_commit_repo("no-greentopo-repo");
    let out = run_without_greentopo(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
        ],
        &bin,
        None,
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let brief = String::from_utf8_lossy(&out.stdout);
    assert!(!brief.contains("<<<RULES"), "{brief}");
    assert!(brief.contains("# Code rules"), "{brief}");
    assert!(
        brief.contains("greentopo is not installed, so its decided code rules were not read"),
        "{brief}"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    let greentopo_notices = err
        .lines()
        .filter(|line| line.contains("has .adr.yaml but greentopo is not installed"))
        .count();
    assert_eq!(greentopo_notices, 1, "{err}");
    assert_eq!(
        err.trim().lines().count(),
        2,
        "the greentopo and greenspotter notices: {err}"
    );
    let sha = run_without_greentopo(&["rules-sha", "-C", repo.to_str().unwrap()], &bin, None);
    assert_eq!(sha.status.code(), Some(3), "{sha:?}");
}

#[cfg(unix)]
#[test]
fn an_greentopo_that_points_nowhere_exits_3() {
    let bin = scratch("greentopo-nowhere");
    let repo = two_commit_repo("greentopo-nowhere-repo");
    let out = run_without_greentopo(
        &["review-brief", "-C", repo.to_str().unwrap()],
        &bin,
        Some("/nonexistent/greentopo"),
    );
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
}

#[cfg(unix)]
#[test]
fn a_non_executable_greentopo_on_path_exits_3() {
    use std::os::unix::fs::PermissionsExt;
    let bin = scratch("greentopo-not-executable");
    let repo = two_commit_repo("greentopo-not-executable-repo");
    std::fs::write(bin.join("greentopo"), "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(
        bin.join("greentopo"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let out = run_without_greentopo(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
        ],
        &bin,
        None,
    );
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
}

#[cfg(unix)]
#[test]
fn a_dangling_greentopo_symlink_on_path_exits_3() {
    let bin = scratch("greentopo-dangling");
    let repo = two_commit_repo("greentopo-dangling-repo");
    std::os::unix::fs::symlink("/nonexistent/target", bin.join("greentopo")).unwrap();
    let out = run_without_greentopo(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
        ],
        &bin,
        None,
    );
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
}

#[cfg(unix)]
#[test]
fn an_greentopo_with_a_missing_interpreter_exits_3() {
    let bin = scratch("greentopo-bad-shebang");
    let repo = two_commit_repo("greentopo-bad-shebang-repo");
    fake_greentopo(&bin, "#!/nonexistent/sh\nexit 0\n");
    let out = run_without_greentopo(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
        ],
        &bin,
        None,
    );
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
}

/// An `greentopo` whose code rules carry `Reviewed by:` lines.
#[cfg(unix)]
const PROFILE_GREENTOPO: &str = r#"#!/bin/sh
case "$*" in
  "--json rules --level constraint --governs code --full")
    printf '%s' '{"rules":[{"id":"cli.unix-only","level":"constraint","statement":"S1.","body":"UNIX-RULE-TEXT\n\n**Checked by:** judge.\n\n**Reviewed by:** unix, tui.","active":true},{"id":"errors.backend-only","level":"constraint","statement":"S2.","body":"BACKEND-RULE-TEXT\n\n**Checked by:** judge.\n\n**Reviewed by:** backend.","active":true},{"id":"types.mixed","level":"constraint","statement":"S3.","body":"MIXED-RULE-TEXT\n\n**Checked by:** exact (a lint), judge (the rest).\n\n**Reviewed by:** unix.","active":true},{"id":"errors.typed","level":"constraint","statement":"S4.","body":"LANGUAGE-RULE-TEXT\n\n**Checked by:** judge.\n\n**Reviewed by:** language.","active":true}]}' ;;
  "rules --level constraint") printf 'constraint x\n' ;;
  "--json relevant"*) printf '{"keys":[]}' ;;
  *) exit 1 ;;
esac
"#;

#[cfg(unix)]
fn profile_brief(name: &str, profile: &str, greentopo: &str) -> (Output, String) {
    let bin = scratch(name);
    fake_greentopo(&bin, greentopo);
    let repo = two_commit_repo(&format!("{name}-repo"));
    let out = run(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
            "--profile",
            profile,
        ],
        &bin,
    );
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    (out, text)
}

#[cfg(unix)]
#[test]
fn a_unix_profile_holds_the_judge_rules_unix_reviews() {
    let (out, brief) = profile_brief("profile-unix", "unix", PROFILE_GREENTOPO);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(brief.contains("## Judge rules for unix"), "{brief}");
    assert!(brief.contains("Profile: unix\n"), "{brief}");
    let judge = &brief[brief.find("## Judge rules for unix").unwrap()..];
    let judge = &judge[..judge.find("# Constraint rules").unwrap()];
    assert!(judge.contains("UNIX-RULE-TEXT"), "{judge}");
    assert!(!judge.contains("BACKEND-RULE-TEXT"), "{judge}");
    assert!(!judge.contains("MIXED-RULE-TEXT"), "{judge}");
    assert!(!judge.contains("LANGUAGE-RULE-TEXT"), "{judge}");
}

/// A profile seat reads its own rules in full and an id index of the rest
/// (ADR-0065); implementation-review still reads every code rule in full.
#[cfg(unix)]
#[test]
fn a_profile_brief_carries_its_rules_and_an_id_index_only() {
    let (out, brief) = profile_brief("profile-slim", "unix", PROFILE_GREENTOPO);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!brief.contains("# Code rules (in full)"), "{brief}");
    assert!(!brief.contains("BACKEND-RULE-TEXT"), "{brief}");
    assert!(brief.contains("UNIX-RULE-TEXT"), "{brief}");
    assert!(brief.contains("# Constraint rules (index)"), "{brief}");
    assert!(brief.contains("greentopo rule <id>"), "{brief}");
    assert!(
        brief.contains("<<<RULES sha="),
        "the RULES line stays: {brief}"
    );
    let bin = scratch("profile-slim-ir");
    fake_greentopo(&bin, PROFILE_GREENTOPO);
    let repo = two_commit_repo("profile-slim-ir-repo");
    let (_, full) = brief_text(&bin, &repo);
    assert!(full.contains("# Code rules (in full)"), "{full}");
    assert!(full.contains("BACKEND-RULE-TEXT"), "{full}");
}

#[cfg(unix)]
#[test]
fn a_lang_profile_holds_the_language_rules() {
    let (out, brief) = profile_brief("profile-lang", "lang:rust", PROFILE_GREENTOPO);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(brief.contains("## Judge rules for lang:rust"), "{brief}");
    let judge = &brief[brief.find("## Judge rules for lang:rust").unwrap()..];
    assert!(judge.contains("LANGUAGE-RULE-TEXT"), "{judge}");
}

#[cfg(unix)]
#[test]
fn an_unknown_reviewed_by_name_exits_1_and_an_unknown_profile_exits_2() {
    let greentopo =
        PROFILE_GREENTOPO.replace("**Reviewed by:** backend.", "**Reviewed by:** wizard.");
    let (out, _) = profile_brief("profile-bad-name", "unix", &greentopo);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("errors.backend-only") && err.contains("wizard"),
        "{err}"
    );
    let (out, _) = profile_brief("profile-unknown", "wizard", PROFILE_GREENTOPO);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

/// The drift guard runs on implementation-review's brief too, not only a
/// profile's: an unknown `Reviewed by:` name exits 1 without `--profile`.
#[cfg(unix)]
#[test]
fn an_unknown_reviewed_by_name_fails_the_brief_without_a_profile() {
    let bin = scratch("generic-bad-name");
    fake_greentopo(
        &bin,
        &PROFILE_GREENTOPO.replace("**Reviewed by:** backend.", "**Reviewed by:** wizard."),
    );
    let repo = two_commit_repo("generic-bad-name-repo");
    let out = run(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
        ],
        &bin,
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("errors.backend-only") && err.contains("wizard"),
        "{err}"
    );
}

/// Candidates are implementation-review's alone: a profile's brief carries
/// neither the CANDIDATES block nor the candidates section.
#[cfg(unix)]
#[test]
fn a_profile_brief_carries_no_candidates() {
    let (out, brief) = profile_brief("profile-no-candidates", "unix", PROFILE_GREENTOPO);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!brief.contains("<<<CANDIDATES"), "{brief}");
    assert!(!brief.contains("## Candidates"), "{brief}");
    assert!(brief.contains("Profile: unix\n"), "{brief}");
}

#[cfg(unix)]
#[test]
fn the_plan_section_carries_the_declared_code_review() {
    let bin = scratch("declared");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let repo = two_commit_repo("declared-repo");
    let name = repo.file_name().unwrap().to_str().unwrap();
    let plan = repo.join("plan.md");
    std::fs::write(
        &plan,
        format!("# P\n\n## Code review\n\n- {name}: none — docs only\n"),
    )
    .unwrap();
    let args = |plan: &PathBuf| {
        run(
            &[
                "review-brief",
                "-C",
                repo.to_str().unwrap(),
                "--base",
                "HEAD~1",
                "--plan",
                plan.to_str().unwrap(),
            ],
            &bin,
        )
    };
    let out = args(&plan);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let brief = String::from_utf8_lossy(&out.stdout);
    assert!(
        brief.contains(&format!(
            "Declared code review for {name}: none — docs only"
        )),
        "{brief}"
    );
    std::fs::write(&plan, "# P\n").unwrap();
    let brief = String::from_utf8_lossy(&args(&plan).stdout).to_string();
    assert!(
        brief.contains(&format!("Declared code review for {name}: not declared")),
        "{brief}"
    );
}

#[cfg(unix)]
#[test]
fn a_declaration_that_does_not_parse_exits_1() {
    let bin = scratch("declared-bad");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let repo = two_commit_repo("declared-bad-repo");
    let name = repo.file_name().unwrap().to_str().unwrap();
    let plan = repo.join("plan.md");
    std::fs::write(
        &plan,
        format!("# P\n\n## Code review\n\n- {name}: wizard\n"),
    )
    .unwrap();
    let out = run(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
            "--plan",
            plan.to_str().unwrap(),
        ],
        &bin,
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        out.stdout.is_empty(),
        "nothing on stdout when the brief fails"
    );
    assert!(
        stderr.contains("## Code review") && stderr.contains("wizard"),
        "{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn a_profile_without_a_corpus_says_it_has_no_rules() {
    let bin = scratch("profile-nocorpus");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let repo = two_commit_repo_with("profile-nocorpus-repo", &[]);
    let out = run(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
            "--profile",
            "unix",
        ],
        &bin,
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let brief = String::from_utf8_lossy(&out.stdout);
    assert!(
        brief.contains("## Judge rules for unix\n\nNone: this repository has no greentopo corpus."),
        "{brief}"
    );
}

/// A branch `feat` off `main`: a reviewed commit, then a fix on top. Returns
/// the repository and the reviewed head.
#[cfg(unix)]
fn delta_repo(name: &str, fix: &[(&str, &str)]) -> (PathBuf, String) {
    let repo = scratch(name);
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join(".adr.yaml"), "x: 1\n").unwrap();
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    git(&repo, &["checkout", "-qb", "feat"]);
    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    git(&repo, &["commit", "-qam", "reviewed"]);
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap();
    let reviewed = String::from_utf8(out.stdout).unwrap().trim().to_string();
    for (path, body) in fix {
        std::fs::write(repo.join(path), body).unwrap();
    }
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fix"]);
    (repo, reviewed)
}

/// `review-brief --interdiff <head>` with the fake greentopo and a private store.
#[cfg(unix)]
fn interdiff(bin: &std::path::Path, repo: &std::path::Path, head: &str) -> Output {
    interdiff_with(bin, repo, head, &[])
}

#[cfg(unix)]
fn interdiff_with(
    bin: &std::path::Path,
    repo: &std::path::Path,
    head: &str,
    more: &[&str],
) -> Output {
    interdiff_env(bin, repo, head, more, &[])
}

#[cfg(unix)]
fn interdiff_env(
    bin: &std::path::Path,
    repo: &std::path::Path,
    head: &str,
    more: &[&str],
    extra_env: &[(&str, &str)],
) -> Output {
    let mut cmd = greenbelay_command();
    cmd.args([
        "review-brief",
        "-C",
        repo.to_str().unwrap(),
        "--base",
        "main",
        "--interdiff",
        head,
    ]);
    cmd.args(more);
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![bin.to_path_buf()];
    dirs.extend(std::env::split_paths(&inherited));
    cmd.env("PATH", std::env::join_paths(dirs).unwrap());
    cmd.env("GREENBELAY_GREENSPOTTER", bin.join("greenspotter"));
    cmd.env("CLAUDE_CONFIG_DIR", bin.join("config"));
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd.output().expect("greenbelay runs")
}

#[cfg(unix)]
#[test]
fn an_interdiff_brief_carries_the_headers_and_only_what_changed() {
    let bin = scratch("interdiff-bin");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let (repo, reviewed) = delta_repo("interdiff", &[("a.txt", "three\n")]);
    let out = interdiff(&bin, &repo, &reviewed);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert!(text.contains("<<<TREE repo="), "{text}");
    assert!(text.contains("<<<RULES sha="), "{text}");
    assert!(text.contains("## Interdiff since"), "{text}");
    assert!(
        text.contains("+three"),
        "the fix is in the interdiff: {text}"
    );
    assert!(!text.contains("## Diff stat"), "no full diff: {text}");
    assert!(!text.contains("## Code rules"), "no rules text: {text}");
}

#[cfg(unix)]
#[test]
fn an_interdiff_is_refused_never_cut() {
    let bin = scratch("interdiff-refused-bin");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let refused = |out: &Output, why: &str| {
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(out.stdout.is_empty(), "{out:?}");
        assert!(
            err.contains(why) && err.contains("without --interdiff"),
            "{err}"
        );
    };
    // A head this repository cannot read.
    let (repo, _) = delta_repo("interdiff-unknown", &[("a.txt", "three\n")]);
    refused(&interdiff(&bin, &repo, &"0".repeat(40)), "is not a commit");
    // The rule files changed since the reviewed head.
    let (repo, reviewed) = delta_repo("interdiff-rules", &[("AGENTS.md", "# new rules\n")]);
    refused(&interdiff(&bin, &repo, &reviewed), "rule files changed");
    // An interdiff of 60 KB or more.
    let big = "x".repeat(70 * 1024) + "\n";
    let (repo, reviewed) = delta_repo("interdiff-big", &[("big.txt", big.as_str())]);
    refused(&interdiff(&bin, &repo, &reviewed), "60 KB or more");
}

#[cfg(unix)]
#[test]
fn an_interdiff_from_another_head_than_the_last_pass_is_refused() {
    let bin = scratch("interdiff-branch-bin");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let (repo, reviewed) = delta_repo("interdiff-branch", &[("a.txt", "three\n")]);
    let name = repo.file_name().unwrap().to_string_lossy().to_string();
    let key = greenbelay_guard::sha256::hex(&greenbelay_guard::sha256::sha256(b"refs/heads/feat"));
    let dir = bin
        .join("config")
        .join("greenbelay")
        .join("implementation-review")
        .join("by-branch")
        .join(&name)
        .join("implementation-review");
    std::fs::create_dir_all(&dir).unwrap();
    let other = "1".repeat(40);
    std::fs::write(
        dir.join(format!("{key}.json")),
        format!(
            r#"{{"head":"{other}","tree":"{}","base":"{}"}}"#,
            "2".repeat(64),
            "3".repeat(40)
        ),
    )
    .unwrap();
    let out = interdiff(&bin, &repo, &reviewed);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        err.contains("last passed review read `111111111111`")
            && err.contains("--interdiff 111111111111"),
        "{err}"
    );
    // A profile seat reads its own record, not the implementation review's.
    let out = interdiff_with(&bin, &repo, &reviewed, &["--profile", "backend"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{out:?}");
    assert!(
        err.contains("no passed review of this branch is recorded"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn an_interdiff_head_that_is_not_hex_is_a_usage_error() {
    let bin = scratch("interdiff-usage-bin");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let (repo, _) = delta_repo("interdiff-usage", &[("a.txt", "three\n")]);
    for value in ["-x", "HEAD~1", "abc", ""] {
        let out = interdiff(&bin, &repo, value);
        assert_eq!(out.status.code(), Some(2), "{value}: {out:?}");
        assert!(out.stdout.is_empty(), "{out:?}");
    }
}

#[cfg(unix)]
#[test]
fn an_interdiff_after_a_rebase_shows_what_the_rebase_changed_in_the_patch() {
    let bin = scratch("interdiff-rebase-bin");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let (repo, reviewed) = delta_repo("interdiff-rebase", &[("a.txt", "three\n")]);
    // `main` moves on; the branch is rebased onto it, so the reviewed head
    // is no longer an ancestor of HEAD.
    git(&repo, &["checkout", "-q", "main"]);
    std::fs::write(repo.join("b.txt"), "main moved\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "main"]);
    git(&repo, &["checkout", "-q", "feat"]);
    git(&repo, &["rebase", "-q", "main"]);
    let out = interdiff(&bin, &repo, &reviewed);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert!(
        text.contains("between its branch patch and HEAD's"),
        "{text}"
    );
    assert!(
        text.contains("three"),
        "the fix is in the interdiff: {text}"
    );
    assert!(
        !text.contains("main moved"),
        "main's own change is not the branch's: {text}"
    );
}

/// `review-brief --profile <p>` on `repo` against `base`, with `more` args.
#[cfg(unix)]
fn brief_for(
    bin: &std::path::Path,
    repo: &std::path::Path,
    base: &str,
    profile: &str,
    more: &[&str],
) -> (Output, String, String) {
    let mut args = vec![
        "review-brief",
        "-C",
        repo.to_str().unwrap(),
        "--base",
        base,
        "--profile",
        profile,
    ];
    args.extend(more);
    let out = run(&args, bin);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    (out, text, err)
}

/// The patch part of a brief: from `## Diff\n` to the next heading.
#[cfg(unix)]
fn patch_of(brief: &str) -> &str {
    let start = brief.find("## Diff\n").expect("a ## Diff section");
    let rest = &brief[start + 8..];
    &rest[..rest
        .find("\n## ")
        .or_else(|| rest.find("\n# "))
        .unwrap_or(rest.len())]
}

#[cfg(unix)]
#[test]
fn a_language_profile_reads_its_own_files_with_the_whole_stat() {
    let bin = scratch("slice-lang-bin");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let repo = two_commit_repo("slice-lang");
    let plan = repo.join("plan.md");
    std::fs::write(
        &plan,
        "# P\n\n## Goal\n\nPURPOSE-TEXT\n\n## Behaviour\n\nBEHAVIOUR-TEXT\n\n## Non-goals\n\nNONGOAL-TEXT\n",
    )
    .unwrap();
    let plan_arg = ["--plan", plan.to_str().unwrap()];
    let (out, brief, err) = brief_for(&bin, &repo, "HEAD~1", "lang:typescript", &plan_arg);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let patch = patch_of(&brief);
    assert!(patch.contains("+export const two = 2;"), "{patch}");
    assert!(
        !patch.contains("pub fn two") && !patch.contains("+two"),
        "{patch}"
    );
    let stat = &brief[brief.find("## Diff stat").unwrap()..brief.find("## Diff\n").unwrap()];
    for path in ["a.txt", "src/lib.rs", "web/app.ts"] {
        assert!(stat.contains(path), "the stat stays whole: {stat}");
    }
    assert!(
        brief.contains("## Not in this brief\n\nChanged, and not of lang:typescript's types, so their patch is not here (the stat lists them):\n\n- `a.txt`\n- `src/lib.rs`\n"),
        "{brief}"
    );
    for text in ["PURPOSE-TEXT", "BEHAVIOUR-TEXT", "NONGOAL-TEXT"] {
        assert!(brief.contains(text), "{text}: {brief}");
    }
    assert!(
        err.contains("lang:typescript: 1 of 3 changed files in this brief ("),
        "{err}"
    );

    // A profile that reads the whole diff: every file, nothing listed out.
    let (out, brief, _) = brief_for(&bin, &repo, "HEAD~1", "backend", &plan_arg);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let patch = patch_of(&brief);
    assert!(
        patch.contains("pub fn two") && patch.contains("export const two"),
        "{patch}"
    );
    assert!(!brief.contains("## Not in this brief"), "{brief}");
    assert!(
        brief.contains("PURPOSE-TEXT"),
        "every profile brief has the Goal"
    );

    // Implementation review's brief is unchanged: no Goal, the whole diff.
    let out = run(
        &[
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
            "--plan",
            plan.to_str().unwrap(),
        ],
        &bin,
    );
    let brief = String::from_utf8_lossy(&out.stdout);
    assert!(
        !brief.contains("PURPOSE-TEXT") && brief.contains("NONGOAL-TEXT"),
        "{brief}"
    );
}

#[cfg(unix)]
#[test]
fn a_profile_with_no_file_of_its_types_gets_no_patch_never_the_whole_diff() {
    let bin = scratch("slice-empty-bin");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let repo = two_commit_repo("slice-empty");
    let (out, brief, err) = brief_for(&bin, &repo, "HEAD~1", "platform", &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        brief.contains(
            "## Diff\n\nNo changed file is of platform's types: this brief carries no patch.\n"
        ),
        "{brief}"
    );
    assert!(!brief.contains("```diff"), "{brief}");
    assert!(brief.contains("- `web/app.ts`"), "{brief}");
    assert!(
        err.contains("platform: 0 of 3 changed files in this brief (0 bytes of patch)"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn more_paths_than_one_git_call_takes_are_filtered_in_batches() {
    let bin = scratch("slice-argv-bin");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let repo = scratch("slice-argv");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    // 1500 paths of about 100 bytes: past the 128 KiB a call may spend.
    let dir = repo.join("web").join("a".repeat(60));
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..1500 {
        std::fs::write(
            dir.join(format!("component_{i:04}_with_a_long_name.ts")),
            format!("export const n = {i};\n"),
        )
        .unwrap();
    }
    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "many"]);
    let (out, _, err) = brief_for(&bin, &repo, "HEAD~1", "lang:typescript", &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    // The batches, joined, are byte for byte the one diff git would print.
    let whole = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["diff", "--no-renames", "HEAD~1...HEAD", "--", "web/"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        err.contains(&format!(
            "lang:typescript: 1500 of 1501 changed files in this brief ({} bytes of patch)",
            whole.stdout.len()
        )),
        "{err}"
    );
}

/// The patch inside a delta brief's ```` ```diff ```` fence.
#[cfg(unix)]
fn fenced(brief: &str) -> &str {
    let start = brief.find("```diff\n").expect("a diff fence") + 8;
    let rest = &brief[start..];
    &rest[..rest.find("\n```").unwrap_or(rest.len())]
}

/// A repository whose branch moves `src/old.rs` to `web/new.ts`, the same
/// text, on top of a reviewed commit. Returns it and the reviewed head.
#[cfg(unix)]
fn rename_repo(name: &str) -> (PathBuf, String) {
    let repo = scratch(name);
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join(".adr.yaml"), "x: 1\n").unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::create_dir_all(repo.join("web")).unwrap();
    let text = "// MOVED-TEXT one\n// two\n// three\n// four\n";
    std::fs::write(repo.join("src/old.rs"), text).unwrap();
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    git(&repo, &["checkout", "-qb", "feat"]);
    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    git(&repo, &["commit", "-qam", "reviewed"]);
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap();
    let reviewed = String::from_utf8(out.stdout).unwrap().trim().to_string();
    git(&repo, &["mv", "src/old.rs", "web/new.ts"]);
    git(&repo, &["commit", "-qm", "move"]);
    (repo, reviewed)
}

#[cfg(unix)]
#[test]
fn a_move_across_languages_shows_each_language_its_side() {
    let bin = scratch("slice-rename-bin");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let sides = |brief: &str| (brief.contains("src/old.rs"), brief.contains("web/new.ts"));

    // The full brief.
    let (repo, _) = rename_repo("slice-rename-full");
    let (out, rust, _) = brief_for(&bin, &repo, "main", "lang:rust", &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let patch = patch_of(&rust);
    assert!(
        patch.contains("deleted file") && patch.contains("-// MOVED-TEXT one"),
        "{patch}"
    );
    assert_eq!(sides(patch), (true, false), "{patch}");
    let (_, ts, _) = brief_for(&bin, &repo, "main", "lang:typescript", &[]);
    let patch = patch_of(&ts);
    assert!(
        patch.contains("new file") && patch.contains("+// MOVED-TEXT one"),
        "{patch}"
    );
    assert_eq!(sides(patch), (false, true), "{patch}");

    // The delta, the move on top of the reviewed head.
    let (repo, reviewed) = rename_repo("slice-rename-on-top");
    let delta = |profile: &str| {
        let out = interdiff_with(&bin, &repo, &reviewed, &["--profile", profile]);
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let rust = delta("lang:rust");
    let rust_patch = fenced(&rust);
    assert!(rust_patch.contains("-// MOVED-TEXT one"), "{rust}");
    assert_eq!(sides(rust_patch), (true, false), "{rust}");
    assert!(
        rust.contains("files outside lang:rust's types left out"),
        "{rust}"
    );
    let ts = delta("lang:typescript");
    let ts_patch = fenced(&ts);
    assert!(ts_patch.contains("+// MOVED-TEXT one"), "{ts}");
    assert!(
        ts_patch.contains("web/new.ts") && !ts_patch.contains("diff --git a/src/old.rs"),
        "{ts}"
    );

    // The delta after a rebase: each branch patch is filtered on its own.
    let (repo, reviewed) = rename_repo("slice-rename-rebased");
    git(&repo, &["checkout", "-q", "main"]);
    std::fs::write(repo.join("b.txt"), "main moved\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "main"]);
    git(&repo, &["checkout", "-q", "feat"]);
    git(&repo, &["rebase", "-q", "main"]);
    let delta = |profile: &str| {
        let out = interdiff_with(&bin, &repo, &reviewed, &["--profile", profile]);
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let rust = delta("lang:rust");
    assert!(
        rust.contains("between its branch patch and HEAD's"),
        "{rust}"
    );
    let rust_patch = fenced(&rust);
    assert!(
        rust_patch.contains("+-// MOVED-TEXT one"),
        "the deletion, now in the patch: {rust}"
    );
    assert!(!rust_patch.contains("+// MOVED-TEXT one"), "{rust}");
    let ts = delta("lang:typescript");
    let ts_patch = fenced(&ts);
    assert!(
        ts_patch.contains("++// MOVED-TEXT one"),
        "the addition, now in the patch: {ts}"
    );
    assert!(!ts_patch.contains("main moved") && !rust_patch.contains("main moved"));
}

/// A repository whose last commit adds a `# release-pin:` marker, registered
/// as a reviewed block in an isolated journal.
#[cfg(unix)]
struct PinWorld {
    bin: PathBuf,
    repo: PathBuf,
    fingerprint: String,
}

#[cfg(unix)]
impl PinWorld {
    fn new(name: &str) -> PinWorld {
        let bin = scratch(&format!("{name}-bin"));
        fake_greentopo(&bin, BRIEF_GREENTOPO);
        let repo = scratch(&format!("{name}-repo"));
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("ci.yml"), "env:\n  TOOL_VERSION: \"1.2\"\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "base"]);
        std::fs::write(
            repo.join("ci.yml"),
            "env:\n  TOOL_VERSION: \"1.2\"\n  # release-pin: o/r v{version} tool-{version}.tgz TOOL_VERSION\n  run: x\n",
        )
        .unwrap();
        git(&repo, &["commit", "-qam", "pin"]);
        let fingerprint = greenbelay_process::block_fingerprint::fingerprint(
            &repo,
            "HEAD~1",
            "HEAD",
            std::time::Instant::now() + std::time::Duration::from_secs(10),
        )
        .unwrap()
        .unwrap()
        .full()
        .to_string();
        greenbelay_process::block_registry::register_entry(
            &bin.join("config/greenbelay"),
            &fingerprint,
            &greenbelay_process::block_registry::Generation {
                rules_sha: "c".repeat(64),
                agent_digest: "d".repeat(64),
            },
            &greenbelay_process::block_registry::EntryData {
                roles: vec!["backend".into()],
                agents: vec![],
                source_branch: "topic".into(),
                source_repo: None,
                content_id: "e".repeat(64),
            },
        )
        .unwrap();
        PinWorld {
            bin,
            repo,
            fingerprint,
        }
    }

    fn gh(&self, body: &str) -> PathBuf {
        let path = self.bin.join("gh-fake");
        let staged = self.bin.join("gh-fake.new");
        std::fs::write(&staged, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&staged, &path).unwrap();
        path
    }

    /// `review-brief`, with `gh` as the override or, for `None`, a PATH that
    /// holds git and the fake greentopo and nothing else.
    fn brief(&self, gh: Option<&std::path::Path>, deadline_ms: &str) -> Output {
        let mut cmd = greenbelay_command();
        cmd.args(["review-brief", "-C"])
            .arg(&self.repo)
            .args(["--base", "HEAD~1"]);
        cmd.env("GREENBELAY_GREENSPOTTER", self.bin.join("greenspotter"));
        cmd.env("CLAUDE_CONFIG_DIR", self.bin.join("config"));
        cmd.env("GREENBELAY_GH_DEADLINE_MS", deadline_ms);
        cmd.env_remove("GREENBELAY_GH");
        match gh {
            Some(gh) => {
                let inherited = std::env::var_os("PATH").unwrap_or_default();
                let mut dirs = vec![self.bin.clone()];
                dirs.extend(std::env::split_paths(&inherited));
                cmd.env("PATH", std::env::join_paths(dirs).unwrap());
                cmd.env("GREENBELAY_GH", gh);
            }
            None => {
                let only = self.bin.join("only-git");
                std::fs::create_dir_all(&only).unwrap();
                let git_path =
                    String::from_utf8(Command::new("which").arg("git").output().unwrap().stdout)
                        .unwrap();
                let _ = std::fs::remove_file(only.join("git")); // a link from an earlier call; absence is the goal
                std::os::unix::fs::symlink(git_path.trim(), only.join("git")).unwrap();
                let _ = std::fs::remove_file(only.join("greentopo")); // same
                std::os::unix::fs::symlink(self.bin.join("greentopo"), only.join("greentopo"))
                    .unwrap();
                cmd.env("PATH", &only);
            }
        }
        cmd.output().unwrap()
    }

    fn recorded(&self) -> Option<greenbelay_process::block_registry::PinRecord> {
        let tree = greenbelay_process::content_id::content_id(&self.repo, "HEAD").unwrap();
        greenbelay_process::block_registry::read_pin(
            &self.bin.join("config/greenbelay"),
            &self.fingerprint,
            &tree,
        )
    }
}

#[cfg(unix)]
const GH_HAS: &str = "#!/bin/sh\nprintf '{\"assets\":[{\"name\":\"tool-1.2.tgz\"}]}'\n";
#[cfg(unix)]
const GH_LACKS: &str = "#!/bin/sh\nprintf '{\"assets\":[{\"name\":\"other.tgz\"}]}'\n";

#[cfg(unix)]
#[test]
fn a_listed_asset_passes_the_pin_check_and_writes_the_record() {
    let world = PinWorld::new("pin-pass");
    let gh = world.gh(GH_HAS);
    let out = world.brief(Some(&gh), "10000");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("## Pin check"), "{text}");
    assert!(text.contains("passed"), "{text}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("gh release view v1.2 --repo o/r"), "{err}");
    assert!(err.contains("pin check passed"), "{err}");
    let pin = world.recorded().expect("a pin record");
    assert!(pin.is_passed(), "{pin:?}");
    assert!(pin.gh_override_set, "{pin:?}");
}

#[cfg(unix)]
#[test]
fn a_missing_journal_directory_is_said_in_the_pin_section() {
    let world = PinWorld::new("pin-no-journal");
    let gh = world.gh(GH_HAS);
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![world.bin.clone()];
    dirs.extend(std::env::split_paths(&inherited));
    let out = greenbelay_command()
        .args(["review-brief", "-C"])
        .arg(&world.repo)
        .args(["--base", "HEAD~1"])
        .env("GREENBELAY_GREENSPOTTER", world.bin.join("greenspotter"))
        .env("PATH", std::env::join_paths(dirs).unwrap())
        .env("GREENBELAY_GH", &gh)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("## Pin check"), "{text}");
    assert!(text.contains("no journal directory"), "{text}");
    assert!(world.recorded().is_none());
}

#[cfg(unix)]
#[test]
fn a_failed_pin_check_overwrites_an_earlier_pass() {
    let world = PinWorld::new("pin-last-wins");
    let gh = world.gh(GH_HAS);
    world.brief(Some(&gh), "10000");
    assert!(world.recorded().unwrap().is_passed());
    let gh = world.gh(GH_LACKS);
    let out = world.brief(Some(&gh), "10000");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("FAILED"), "{text}");
    let pin = world.recorded().unwrap();
    assert!(!pin.is_passed(), "{pin:?}");
    assert!(
        pin.detail().contains("does not list tool-1.2.tgz"),
        "{pin:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_failing_slow_or_absent_gh_writes_a_failure() {
    let world = PinWorld::new("pin-gh-broken");
    let failing = world.gh("#!/bin/sh\necho boom >&2\nexit 1\n");
    let out = world.brief(Some(&failing), "10000");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let pin = world.recorded().unwrap();
    assert!(
        !pin.is_passed() && pin.detail().contains("failed (exit 1)"),
        "{pin:?}"
    );

    let hanging = world.gh("#!/bin/sh\nexec sleep 30\n");
    let out = world.brief(Some(&hanging), "300");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let pin = world.recorded().unwrap();
    assert!(
        !pin.is_passed() && pin.detail().contains("timed out"),
        "{pin:?}"
    );

    let out = world.brief(None, "10000");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let pin = world.recorded().unwrap();
    assert!(!pin.is_passed(), "{pin:?}");
    assert_eq!(
        pin.detail(),
        "gh not found; install GitHub CLI and run gh auth login"
    );
    assert!(!pin.gh_override_set);
}

#[cfg(unix)]
#[test]
fn a_branch_that_is_not_a_registered_block_has_no_pin_section() {
    let world = PinWorld::new("pin-unregistered");
    std::fs::write(world.repo.join("other.txt"), "x\n").unwrap();
    git(&world.repo, &["add", "."]);
    git(&world.repo, &["commit", "-qm", "more"]);
    let gh = world.gh(GH_HAS);
    let out = world.brief(Some(&gh), "10000");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("## Pin check"));
    assert!(world.recorded().is_none());
}

/// The greenbelay binary pointed at scratch directories, so the migration every
/// subcommand runs first never touches the real home of whoever runs the tests.
/// A caller that sets one of these variables itself overrides it afterwards.
fn greenbelay_command() -> Command {
    let scratch = fresh_scratch("review_brief");
    let mut command = Command::new(env!("CARGO_BIN_EXE_greenbelay"));
    command
        .env("CLAUDE_CONFIG_DIR", scratch.join("claude"))
        .env("XDG_STATE_HOME", scratch.join("state"))
        .env("GIT_CONFIG_GLOBAL", scratch.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

/// A new, empty scratch directory for one spawn, named by process id, the
/// clock and a retry counter; `create_dir` fails on a name already taken.
fn fresh_scratch(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mut suffix = 0u32;
    loop {
        let dir = std::env::temp_dir().join(format!(
            "gp-{label}-{}-{nanos}-{suffix}",
            std::process::id()
        ));
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                std::fs::create_dir(dir.join("claude")).expect("scratch config dir");
                std::fs::create_dir(dir.join("state")).expect("scratch state home");
                return dir;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => suffix += 1,
            Err(error) => panic!("scratch directory {}: {error}", dir.display()),
        }
    }
}

#[cfg(unix)]
#[test]
fn the_retired_adr_lint_variable_gets_a_notice_in_the_full_and_the_delta_brief() {
    let bin = scratch("retired-env");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let repo = two_commit_repo("retired-env-repo");
    let full_brief_with = |adr_lint_value: &str| {
        let mut cmd = greenbelay_command();
        cmd.args([
            "review-brief",
            "-C",
            repo.to_str().unwrap(),
            "--base",
            "HEAD~1",
        ]);
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.clone()];
        dirs.extend(std::env::split_paths(&inherited));
        cmd.env("PATH", std::env::join_paths(dirs).unwrap());
        cmd.env("GREENBELAY_GREENSPOTTER", bin.join("greenspotter"));
        cmd.env("GREENBELAY_ADR_LINT", adr_lint_value);
        cmd.output().expect("greenbelay runs")
    };
    let set = full_brief_with("/nonexistent");
    assert_eq!(set.status.code(), Some(0), "{set:?}");
    let err = String::from_utf8_lossy(&set.stderr);
    assert_eq!(
        err.matches("GREENBELAY_ADR_LINT is no longer read: set GREENBELAY_GREENSPOTTER")
            .count(),
        1,
        "{err}"
    );
    let empty = full_brief_with("");
    let err = String::from_utf8_lossy(&empty.stderr);
    assert!(!err.contains("no longer read"), "{err}");

    let (delta, reviewed) = delta_repo("retired-env-delta", &[("a.txt", "three\n")]);
    let out = interdiff_env(
        &bin,
        &delta,
        &reviewed,
        &[],
        &[("GREENBELAY_ADR_LINT", "x")],
    );
    assert!(out.status.success(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        err.matches("GREENBELAY_ADR_LINT is no longer read").count(),
        1,
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn the_retired_aval_variable_is_not_spawned_and_gets_a_notice() {
    let bin = scratch("retired-aval");
    fake_greentopo(&bin, BRIEF_GREENTOPO);
    let repo = two_commit_repo("retired-aval-repo");
    let mut cmd = greenbelay_command();
    cmd.args([
        "review-brief",
        "-C",
        repo.to_str().unwrap(),
        "--base",
        "HEAD~1",
    ]);
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![bin.clone()];
    dirs.extend(std::env::split_paths(&inherited));
    cmd.env("PATH", std::env::join_paths(dirs).unwrap());
    cmd.env("GREENBELAY_GREENSPOTTER", bin.join("greenspotter"));
    cmd.env("GREENBELAY_AVAL", "/nonexistent/aval");
    let out = cmd.output().expect("greenbelay runs");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        err.matches("GREENBELAY_AVAL is no longer read: set GREENBELAY_GREENTOPO")
            .count(),
        1,
        "{err}"
    );
}
