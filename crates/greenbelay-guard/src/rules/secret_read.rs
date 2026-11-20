//! `secret-read` — a secret's bytes poured into the tool result or copied.
//!
//! ```sh
//! cat ~/.netrc                 # the token is now in the transcript
//! cat ~/.netrc | pbcopy        # a pipe does not hide it from the model's call
//! cp ~/.ssh/id_ed25519 /tmp/k  # a second, unguarded copy
//! ```
//!
//! Advise only: `max_stance` is Advise, so neither `graduate` nor git config
//! can make this refuse. The agent is told once per path, the command still
//! runs, and a legitimate use of a credential by reference (`curl -n`,
//! `curl --cert … --key …`, `ssh`) stays silent because the program name
//! decides: only a reader of bytes is looked at.
//!
//! One matcher serves three tools. [`secret_path`] is pure and judges a path
//! as written; [`secret_file`] looks at the file itself and is what the Bash
//! `confirm`, the Read block and the Grep block of the hook all call, so the
//! three never disagree about what a secret is.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::rules::{Confirmed, Context, Evidence, Examine, Finding, Rule, Stance, Trend};
use crate::shell::{resolve_word, Parsed, Resolved, Simple, Word};

pub const RULE: Rule = Rule {
    id: "secret-read",
    prevents:
        "A secret's bytes poured into the tool result or copied, where the transcript keeps them",
    irreversible: false,
    default_stance: Stance::Advise,
    max_stance: Stance::Advise,
    evidence: Evidence {
        // Unmeasured: the rule ships advise-only before a `greenbelay mine`
        // or `backtest` count over the transcripts exists, so no rate is
        // claimed here.
        per_1000: 0.0,
        measured: "2026-11-20",
        trend: Trend::Rare,
    },
    examine: Examine::Legacy(examine),
    confirm: Some(confirm),
};

/// Programs that put the bytes of a file they are given in front of the model
/// (or, for `cp`, into another file).
const READERS: &[&str] = &[
    "cat", "head", "tail", "sed", "bat", "more", "less", "nl", "grep", "rg", "ugrep", "awk", "xxd",
    "od", "strings", "base64", "jq", "cp",
];

/// Readers whose first operand is a pattern, script or filter, not a path,
/// unless a flag supplied it.
const PATTERN_FIRST: &[&str] = &["grep", "rg", "ugrep", "sed", "awk", "jq"];

/// How much of a candidate key file is read to look for `PRIVATE KEY`.
const KEY_PROBE_BYTES: u64 = 64 * 1024;

/// Labels whose file must hold a `PRIVATE KEY` block to count: a CA bundle
/// named `.pem` is not a secret.
fn needs_content(label: &str) -> bool {
    matches!(label, "private key" | "mtls key")
}

/// Whether the file name is one the rule leaves alone: public halves and
/// templates.
pub fn exempt(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let parent = parent_name(path);
    if name.ends_with(".pub") {
        return true;
    }
    if parent == ".ssh" && (name.starts_with("known_hosts") || name == "config") {
        return true;
    }
    if parent == ".ssh" && name == "authorized_keys" {
        return true;
    }
    name.starts_with(".env")
        && [".example", ".sample", ".template"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn parent_name(path: &Path) -> &str {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .unwrap_or("")
}

/// The kind of secret `shown` names, from the path as written (`~/`,
/// `$HOME/` and absolute forms alike), or `None`. Pure: nothing is opened.
/// A name that needs its content looked at ([`needs_content`]) matches here
/// and is settled by [`secret_file`].
pub fn secret_path(shown: &str) -> Option<&'static str> {
    let path = Path::new(shown);
    if exempt(path) {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    let parent = parent_name(path);
    let grandparent = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let ext_is = |wanted: &str| {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(wanted))
    };
    if parent == ".ssh" {
        return Some("ssh key");
    }
    if parent == "mtls" && grandparent == ".config" && name.ends_with(".key") {
        return Some("mtls key");
    }
    if parent == ".claude" && name.starts_with(".credentials") {
        return Some("Claude credentials");
    }
    if name == ".netrc" {
        return Some("netrc");
    }
    if name == ".npmrc" {
        return Some("npmrc");
    }
    if parent == ".aws" && name == "credentials" {
        return Some("AWS credentials");
    }
    if parent == ".docker" && name == "config.json" {
        return Some("docker credentials");
    }
    if parent == ".kube" && name == "config" {
        return Some("kubeconfig");
    }
    if name == ".envrc" {
        return Some("direnv file");
    }
    if name == ".env"
        || name.starts_with(".env.")
        || (name.starts_with(".env") && name.contains(['*', '?']))
    {
        return Some("env file");
    }
    if ext_is("key") || ext_is("pem") {
        return Some("private key");
    }
    None
}

/// Whether `name` holds a glob character.
fn has_glob(name: &str) -> bool {
    name.contains(['*', '?'])
}

/// `*` and `?` matching of one path component.
fn wildcard(pattern: &[char], text: &[char]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some(('*', rest)) => (0..=text.len()).any(|skip| wildcard(rest, &text[skip..])),
        Some((first, rest)) => text
            .split_first()
            .is_some_and(|(t, tail)| (*first == '?' || first == t) && wildcard(rest, tail)),
    }
}

/// The kind of secret the file at `path` is, or `None`: the name matches
/// [`secret_path`], a glob in the last component is expanded and any entry
/// that is a secret counts, the file exists, and a key file holds a
/// `PRIVATE KEY`. This is the one answer Bash, Read and Grep share.
pub fn secret_file(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    if !has_glob(name) {
        return secret_literal(path);
    }
    let pattern: Vec<char> = name.chars().collect();
    let dir = path.parent()?;
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|n| wildcard(&pattern, &n.chars().collect::<Vec<_>>()))
        })
        .find_map(|entry| secret_literal(&entry.path()))
}

/// [`secret_file`] for a path with no glob in it.
fn secret_literal(path: &Path) -> Option<&'static str> {
    let label = secret_path(path.to_str()?)?;
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    if needs_content(label) && !holds_private_key(path) {
        return None;
    }
    Some(label)
}

fn holds_private_key(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::new();
    if file.take(KEY_PROBE_BYTES).read_to_end(&mut head).is_err() {
        return false;
    }
    String::from_utf8_lossy(&head).contains("PRIVATE KEY")
}

/// A path operand of a reader, as written.
struct PathRead {
    shown: String,
    at: usize,
    end: usize,
    clause_at: usize,
    copy: bool,
}

fn word_path(word: &Word) -> Option<String> {
    let lookup = |name: &str| (name == "HOME").then(|| "~".to_string());
    match resolve_word(word, &lookup) {
        Resolved::Text(text) if !text.is_empty() && !text.starts_with('-') => Some(text),
        Resolved::Text(_) | Resolved::Variable | Resolved::Substitution => None,
    }
}

/// A flag that shows no bytes of the file: `-q -l -c -L`, alone or in a
/// cluster, and their long forms.
fn quiet_flag(program: &str, flag: &str) -> bool {
    if !matches!(program, "grep" | "rg" | "ugrep") {
        return false;
    }
    if flag.starts_with("--") {
        return matches!(
            flag,
            "--quiet" | "--silent" | "--count" | "--files-with-matches" | "--files-without-match"
        ) || (program == "rg" && flag == "--files");
    }
    let cluster = &flag[1..];
    cluster.chars().all(|c| c.is_ascii_alphabetic())
        && cluster
            .chars()
            .any(|c| matches!(c, 'q' | 'l' | 'c') || (c == 'L' && program != "rg"))
}

/// `sed -i`, alone or in a cluster: an edit in place, not a read.
fn in_place(flag: &str) -> bool {
    if flag.starts_with("--") {
        return flag.starts_with("--in-place");
    }
    flag.starts_with("-i")
        || (flag[1..].chars().all(|c| c.is_ascii_alphabetic()) && flag[1..].contains('i'))
}

/// The words of `cmd` that name files this reader reads.
fn read_operands<'a>(cmd: &'a Simple, program: &str) -> Vec<&'a Word> {
    let args = cmd.args();
    let mut words: Vec<&Word> = Vec::new();
    let mut pattern_given = false;
    let mut target_directory_given = false;
    let mut skip = false;
    let mut options_ended = false;
    for word in args {
        if skip {
            skip = false;
            continue;
        }
        let text = word.text.as_str();
        if !options_ended && !word.quoted && text == "--" {
            options_ended = true;
            continue;
        }
        if !options_ended && !word.quoted && text.starts_with('-') && text.len() > 1 {
            if quiet_flag(program, text) || (program == "sed" && in_place(text)) {
                return Vec::new();
            }
            if PATTERN_FIRST.contains(&program) {
                if matches!(text, "-e" | "-f" | "--regexp" | "--file") {
                    pattern_given = true;
                    skip = true;
                } else if text.starts_with("-e")
                    || text.starts_with("-f")
                    || text.starts_with("--regexp=")
                    || text.starts_with("--file=")
                {
                    pattern_given = true;
                }
            }
            if program == "cp" && matches!(text, "-t" | "--target-directory") {
                target_directory_given = true;
                skip = true;
            }
            continue;
        }
        words.push(word);
    }
    if PATTERN_FIRST.contains(&program) && !pattern_given && !words.is_empty() {
        words.remove(0);
    }
    if program == "cp" && !target_directory_given {
        // Only a source is read; the last operand is where the copy goes.
        words.pop();
    }
    words
}

/// Every path a reader in the command reads, in order.
fn reads(parsed: &Parsed) -> Vec<PathRead> {
    let mut out = Vec::new();
    for cmd in parsed.judgeable() {
        // `X=$(cat f)`: the bytes go to the shell, not to the tool result.
        if cmd.nested.is_some() {
            continue;
        }
        let Some(program) = cmd.program() else {
            continue;
        };
        if !READERS.contains(&program) {
            continue;
        }
        let mut words = read_operands(cmd, program);
        if program != "cp" {
            words.extend(
                cmd.redirects
                    .iter()
                    .filter(|(op, _)| op == "<" || op == "0<")
                    .map(|(_, target)| target),
            );
        }
        for word in words {
            if let Some(shown) = word_path(word) {
                out.push(PathRead {
                    shown,
                    at: word.at,
                    end: word.at + word.raw.len(),
                    clause_at: cmd.at,
                    copy: program == "cp",
                });
            }
        }
    }
    out
}

fn examine(parsed: &Parsed) -> Option<Finding> {
    let found = reads(parsed)
        .into_iter()
        .find(|read| secret_path(&read.shown).is_some())?;
    let label = secret_path(&found.shown)?;
    let (reason, remedy) = phrase(&found.shown, label, found.copy);
    Some(Finding {
        reason,
        remedy,
        // The path operand alone: the journal must never hold the command
        // text, which may carry a pattern or a token.
        span: found.at..found.end,
    })
}

/// Where a finding's path is, resolved against the directory its clause runs
/// in. `None` when the command no longer holds it or `~` has no home.
pub fn resolved(context: &Context, finding: &Finding) -> Option<(PathBuf, String, bool)> {
    let read = reads(context.parsed)
        .into_iter()
        .find(|read| read.at == finding.span.start)?;
    let path = if read.shown.starts_with('/') {
        PathBuf::from(&read.shown)
    } else if let Some(rest) = read.shown.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME")?).join(rest)
    } else {
        context.cwd_at(read.clause_at).join(&read.shown)
    };
    Some((path, read.shown, read.copy))
}

fn confirm(context: &Context, finding: &Finding) -> Confirmed {
    let Some((path, shown, copy)) = resolved(context, finding) else {
        return Confirmed::No("the command no longer matches");
    };
    let Some(label) = secret_file(&path) else {
        return Confirmed::No("not there, or not a secret");
    };
    let (reason, remedy) = phrase(&shown, label, copy);
    Confirmed::YesSaying {
        floor: None,
        ceiling: Some(Stance::Advise),
        reason,
        excerpt: excerpt(label, &shown),
        remedy: Some(remedy),
    }
}

/// What the journal keeps: the kind and the path, never the command.
pub fn excerpt(label: &str, shown: &str) -> String {
    format!("{label} {shown}")
}

/// `shown` as one shell word. A leading `~/` stays outside the quotes so the
/// shell still expands it.
fn quoted(shown: &str) -> String {
    let safe = |text: &str| {
        text.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.,:=+@%/".contains(c))
    };
    let (home, rest) = match shown.strip_prefix("~/") {
        Some(rest) => ("~/", rest),
        None => ("", shown),
    };
    if safe(rest) {
        return shown.to_string();
    }
    format!("{home}'{}'", rest.replace('\'', "'\\''"))
}

/// What the hook says: the mechanism, then the way through. The reason is in
/// the future tense because the call has not run yet and, advise only, will.
pub fn phrase(shown: &str, label: &str, copy: bool) -> (String, String) {
    let path = quoted(shown);
    let reason = if copy {
        format!(
            "`{shown}` is a {label}: copying it will leave its bytes in a second file that \
             nothing guards. The call will still run."
        )
    } else {
        format!(
            "`{shown}` is a {label}: its bytes will land in the tool result, and the \
             transcript and every later turn keep them. The call will still run."
        )
    };
    let remedy = if copy {
        format!("Point the tool at `{path}` instead of copying the secret.")
    } else {
        match label {
            "netrc" => format!(
                "Pass it by reference (`curl --netrc-file {path}`, a token in `$TOK`); \
                 check it with `wc -c {path}`."
            ),
            "ssh key" => format!("Use it through `ssh-add {path}`; never print it."),
            "env file" => format!("List key names only: `cut -d= -f1 {path}`."),
            "direnv file" => {
                "Do not print it: name the variable you need and read that one from the \
                 environment."
                    .to_string()
            }
            "private key" | "mtls key" => {
                format!("Pass it by reference (`--key {path}`); never print it.")
            }
            _ => format!(
                "Pass it by reference (the tool's own flag, a token in `$TOK`); check it \
                 with `wc -c {path}`."
            ),
        }
    };
    (reason, remedy)
}

/// Whether the rule fires on one corpus Read case: the same name test the
/// hook's Read block makes, minus the stat and the key probe the corpus
/// cannot do.
pub fn judge_case(case: &crate::corpus::ReadCase) -> bool {
    secret_path(&case.path).is_some_and(|label| !needs_content(label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::lex;

    fn fires(command: &str) -> bool {
        examine(&lex(command)).is_some()
    }

    #[test]
    fn a_reader_of_a_secret_fires() {
        assert!(fires("cat ~/.netrc"));
        assert!(fires("cat ~/.netrc | pbcopy"));
        assert!(fires("grep token ~/.aws/credentials"));
        assert!(fires("cp ~/.ssh/id_ed25519 /tmp/k"));
        assert!(fires("head -3 .env"));
        assert!(fires("cat \"$HOME/.netrc\""));
        assert!(fires("cat ${HOME}/.netrc"));
        assert!(fires("jq . < .env"));
        assert!(fires("cat < ~/.netrc"));
        assert!(fires("grep -e token ~/.netrc"));
        assert!(fires("cat ~/.ssh/id_*"));
    }

    #[test]
    fn a_credential_used_by_reference_is_silent() {
        assert!(!fires("curl -n https://x"));
        assert!(!fires(
            "curl --cert ~/.config/mtls/c.crt --key ~/.config/mtls/c.key https://x"
        ));
        assert!(!fires("ls ~/.ssh"));
        assert!(!fires("ssh-add ~/.ssh/id_ed25519"));
        assert!(!fires("cat ~/.ssh/id_ed25519.pub"));
        assert!(!fires("cat ~/.ssh/known_hosts"));
        assert!(!fires("cat .env.example"));
        assert!(!fires("grep -q x ~/.netrc"));
        assert!(!fires("grep -qi x ~/.netrc"));
        assert!(!fires("grep --count x ~/.netrc"));
        assert!(!fires("grep .env README.md"));
        assert!(!fires("sed -i 's/a/b/' .env"));
        assert!(!fires("cp /tmp/k ~/.ssh/id_ed25519"));
        assert!(!fires("X=$(cat ~/.netrc)"));
        assert!(!fires("cat > ~/.netrc <<'EOF'\nmachine x\nEOF\n"));
    }

    #[test]
    fn the_finding_spans_the_path_alone() {
        let command = "grep ghp_abc ~/.netrc";
        let finding = examine(&lex(command)).expect("fires");
        assert_eq!(&command[finding.span.clone()], "~/.netrc");
        assert!(!finding.reason.contains("ghp_abc"));
        assert!(!finding.remedy.contains("ghp_abc"));
    }

    #[test]
    fn the_journal_excerpt_of_a_finding_never_holds_the_pattern() {
        let command = "grep ghp_abc ~/.netrc";
        let finding = examine(&lex(command)).expect("fires");
        let journalled = crate::backtest::excerpt(command, finding.span.start, finding.span.end);
        assert!(journalled.contains("~/.netrc"), "{journalled}");
        assert!(!journalled.contains("ghp_abc"), "{journalled}");
    }

    #[test]
    fn the_path_matcher_names_each_kind() {
        for (shown, label) in [
            ("~/.ssh/id_ed25519", "ssh key"),
            ("/Users/x/.ssh/id_rsa", "ssh key"),
            ("~/.netrc", "netrc"),
            ("~/.aws/credentials", "AWS credentials"),
            ("~/.docker/config.json", "docker credentials"),
            ("~/.kube/config", "kubeconfig"),
            ("~/.npmrc", "npmrc"),
            ("~/.config/mtls/c.key", "mtls key"),
            ("~/.claude/.credentials.json", "Claude credentials"),
            ("app/.env", "env file"),
            ("app/.env.local", "env file"),
            ("app/.envrc", "direnv file"),
            ("certs/server.pem", "private key"),
            ("certs/server.key", "private key"),
        ] {
            assert_eq!(secret_path(shown), Some(label), "{shown}");
        }
        for shown in [
            "~/.ssh/id_ed25519.pub",
            "~/.ssh/config",
            "~/.ssh/known_hosts",
            "app/.env.example",
            "app/.env.sample",
            "app/.env.template",
            "~/.claude/settings.json",
            "~/.config/mtls/c.crt",
            "src/main.rs",
        ] {
            assert_eq!(secret_path(shown), None, "{shown}");
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("greenbelay-sr-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn a_pem_is_a_secret_only_when_it_holds_a_private_key() {
        let dir = scratch("pem");
        let bundle = dir.join("ca.pem");
        std::fs::write(&bundle, "-----BEGIN CERTIFICATE-----\n").expect("bundle");
        let key = dir.join("server.pem");
        // Built at run time: the header spelled out would trip the secrets scan.
        let header = format!("-----BEGIN {} KEY-----\n", "PRIVATE");
        std::fs::write(&key, header).expect("key");
        assert_eq!(secret_file(&bundle), None);
        assert_eq!(secret_file(&key), Some("private key"));
        assert_eq!(secret_file(&dir.join("missing.pem")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_glob_is_expanded_before_the_exists_check() {
        let dir = scratch("glob");
        let ssh = dir.join(".ssh");
        std::fs::create_dir_all(&ssh).expect("ssh dir");
        assert_eq!(secret_file(&ssh.join("id_*")), None, "nothing there yet");
        std::fs::write(ssh.join("id_ed25519.pub"), "ssh-ed25519 AAAA").expect("pub");
        assert_eq!(secret_file(&ssh.join("id_*")), None, "only a public key");
        std::fs::write(ssh.join("id_ed25519"), "secret").expect("key");
        assert_eq!(secret_file(&ssh.join("id_*")), Some("ssh key"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_is_not_a_secret_file() {
        let dir = scratch("dir");
        let env = dir.join(".env");
        std::fs::create_dir_all(&env).expect("dir named .env");
        assert_eq!(secret_file(&env), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_remedy_quotes_the_path_and_keeps_the_home_prefix() {
        let (reason, remedy) = phrase("/a b/it's/.env", "env file", false);
        assert!(remedy.contains(r#"'/a b/it'\''s/.env'"#), "{remedy}");
        assert!(reason.contains("will land"), "{reason}");
        let (_, home) = phrase("~/.netrc", "netrc", false);
        assert!(home.contains("--netrc-file ~/.netrc"), "{home}");
        let (_, spaced) = phrase("~/my dir/.netrc", "netrc", false);
        assert!(spaced.contains("~/'my dir/.netrc'"), "{spaced}");
        let (copy_reason, copy_remedy) = phrase("~/.ssh/id_rsa", "ssh key", true);
        assert!(copy_reason.contains("copying it will"), "{copy_reason}");
        assert!(copy_remedy.contains("instead of copying"), "{copy_remedy}");
        let (_, direnv) = phrase("app/.envrc", "direnv file", false);
        assert!(direnv.contains("Do not print it"), "{direnv}");
    }

    #[test]
    fn the_rule_can_only_ever_advise() {
        assert_eq!(RULE.default_stance, Stance::Advise);
        assert_eq!(RULE.max_stance, Stance::Advise);
        assert_eq!(
            crate::stance::ladder(&RULE, Some(Stance::Deny)),
            Stance::Advise,
            "git config cannot raise it"
        );
        let verdict = crate::graduate::assess(&RULE, Stance::Deny);
        assert!(!verdict.allowed, "graduate must refuse to raise it");
        assert!(
            verdict
                .lines
                .iter()
                .any(|(_, line)| line.contains("capped at advise")),
            "{:?}",
            verdict.lines
        );
    }
}
