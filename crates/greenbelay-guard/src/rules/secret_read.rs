//! `secret-read` — a secret's contents poured into the tool result or copied.
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
//! decides: only a reader of contents is looked at.
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
        "A secret's contents poured into the tool result or copied, where the transcript keeps them",
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

/// What kind of secret a path is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretKind {
    SshKey,
    MtlsKey,
    ClaudeCredentials,
    Netrc,
    Npmrc,
    AwsCredentials,
    DockerCredentials,
    Kubeconfig,
    DirenvFile,
    EnvFile,
    PrivateKey,
}

impl SecretKind {
    /// The name the advice and the journal use.
    pub fn label(self) -> &'static str {
        match self {
            SecretKind::SshKey => "ssh key",
            SecretKind::MtlsKey => "mtls key",
            SecretKind::ClaudeCredentials => "Claude credentials",
            SecretKind::Netrc => "netrc",
            SecretKind::Npmrc => "npmrc",
            SecretKind::AwsCredentials => "AWS credentials",
            SecretKind::DockerCredentials => "docker credentials",
            SecretKind::Kubeconfig => "kubeconfig",
            SecretKind::DirenvFile => "direnv file",
            SecretKind::EnvFile => "env file",
            SecretKind::PrivateKey => "private key",
        }
    }

    /// Whether the file must hold a `PRIVATE KEY` block to count: a CA bundle
    /// named `.pem` is not a secret.
    fn needs_content(self) -> bool {
        matches!(self, SecretKind::PrivateKey | SecretKind::MtlsKey)
    }
}

/// Programs that put the contents of a file they are given in front of the model
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

/// Whether the file name is one the rule leaves alone: public halves and
/// templates.
fn exempt(path: &Path) -> bool {
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
/// A name that needs its content looked at ([`SecretKind::needs_content`])
/// matches here and is settled by [`secret_file`].
pub fn secret_path(shown: &str) -> Option<SecretKind> {
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
        return Some(SecretKind::SshKey);
    }
    if parent == "mtls" && grandparent == ".config" && name.ends_with(".key") {
        return Some(SecretKind::MtlsKey);
    }
    if parent == ".claude" && name.starts_with(".credentials") {
        return Some(SecretKind::ClaudeCredentials);
    }
    if name == ".netrc" {
        return Some(SecretKind::Netrc);
    }
    if name == ".npmrc" {
        return Some(SecretKind::Npmrc);
    }
    if parent == ".aws" && name == "credentials" {
        return Some(SecretKind::AwsCredentials);
    }
    if parent == ".docker" && name == "config.json" {
        return Some(SecretKind::DockerCredentials);
    }
    if parent == ".kube" && name == "config" {
        return Some(SecretKind::Kubeconfig);
    }
    if name == ".envrc" {
        return Some(SecretKind::DirenvFile);
    }
    if name == ".env"
        || name.starts_with(".env.")
        || (name.starts_with(".env") && name.contains(['*', '?']))
    {
        return Some(SecretKind::EnvFile);
    }
    if ext_is("key") || ext_is("pem") {
        return Some(SecretKind::PrivateKey);
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
pub fn secret_file(path: &Path) -> Option<SecretKind> {
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
fn secret_literal(path: &Path) -> Option<SecretKind> {
    let kind = secret_path(path.to_str()?)?;
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    if kind.needs_content() && !holds_private_key(path) {
        return None;
    }
    Some(kind)
}

fn holds_private_key(path: &Path) -> bool {
    // Fails open: a file that cannot be opened or read is not advised about.
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::new();
    if file.take(KEY_PROBE_BYTES).read_to_end(&mut head).is_err() {
        return false;
    }
    String::from_utf8_lossy(&head).contains("PRIVATE KEY")
}

/// How a reader's operand leaves the file: shown to the model, or copied into
/// another file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Show,
    Copy,
}

/// A path operand of a reader, as written.
struct PathRead {
    shown: String,
    at: usize,
    end: usize,
    clause_at: usize,
    access: Access,
}

fn word_path(word: &Word) -> Option<String> {
    let lookup = |name: &str| (name == "HOME").then(|| "~".to_string());
    match resolve_word(word, &lookup) {
        Resolved::Text(text) if !text.is_empty() && !text.starts_with('-') => Some(text),
        Resolved::Text(_) | Resolved::Variable | Resolved::Substitution => None,
    }
}

/// What one flag word of a reader means for the operands after it.
struct FlagShape {
    /// Shows no contents of the file: `-q -l -c -L` and their long forms.
    quiet: bool,
    /// Supplies the pattern, script or filter (`-e`, `-f`).
    pattern: bool,
    /// Names the target directory of a `cp` (`-t`).
    target_directory: bool,
    /// How many following words are this flag's values.
    skip: usize,
}

/// The short flags of `program` that take an argument, attached (`-A2`) or as
/// the next word (`-A 2`).
fn short_valued(program: &str) -> &'static str {
    match program {
        "grep" | "ugrep" => "efABCmdD",
        "rg" => "efABCmgtTMj",
        "awk" => "Fvf",
        "sed" | "jq" => "ef",
        "cp" => "tS",
        _ => "",
    }
}

/// The long flags of `program` that take their value as the next word, with
/// how many words that is.
fn long_arity(program: &str, flag: &str) -> usize {
    match (program, flag) {
        ("grep" | "ugrep", "--regexp" | "--file")
        | ("grep" | "ugrep", "--after-context" | "--before-context" | "--context")
        | ("grep" | "ugrep", "--max-count" | "--directories" | "--devices")
        | ("grep" | "ugrep", "--exclude" | "--include" | "--exclude-dir") => 1,
        ("rg", "--regexp" | "--file" | "--after-context" | "--before-context" | "--context")
        | ("rg", "--max-count" | "--glob" | "--type" | "--type-not" | "--max-columns")
        | ("rg", "--threads") => 1,
        ("sed", "--expression" | "--file") | ("jq", "--from-file") => 1,
        ("jq", "--arg" | "--argjson" | "--slurpfile" | "--rawfile") => 2,
        ("cp", "--target-directory" | "--suffix") => 1,
        _ => 0,
    }
}

fn flag_shape(program: &str, flag: &str) -> FlagShape {
    let pattern_reader = PATTERN_FIRST.contains(&program);
    if let Some(long) = flag.strip_prefix("--") {
        let name = long.split('=').next().unwrap_or(long);
        let quiet = matches!(
            flag,
            "--quiet" | "--silent" | "--count" | "--files-with-matches" | "--files-without-match"
        ) && matches!(program, "grep" | "rg" | "ugrep");
        let quiet = quiet || (program == "rg" && flag == "--files");
        let attached = long.contains('=');
        let bare = format!("--{name}");
        return FlagShape {
            quiet,
            pattern: pattern_reader
                && matches!(name, "regexp" | "file" | "expression" | "from-file"),
            target_directory: program == "cp" && name == "target-directory",
            skip: if attached {
                0
            } else {
                long_arity(program, &bare)
            },
        };
    }
    let cluster = &flag[1..];
    let mut quiet = false;
    for (offset, letter) in cluster.char_indices() {
        if short_valued(program).contains(letter) {
            let attached = offset + letter.len_utf8() < cluster.len();
            return FlagShape {
                quiet,
                pattern: pattern_reader && matches!(letter, 'e' | 'f'),
                target_directory: program == "cp" && letter == 't',
                skip: usize::from(!attached),
            };
        }
        if matches!(program, "grep" | "rg" | "ugrep")
            && (matches!(letter, 'q' | 'l' | 'c') || (letter == 'L' && program != "rg"))
        {
            quiet = true;
        }
    }
    FlagShape {
        quiet,
        pattern: false,
        target_directory: false,
        skip: 0,
    }
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
    let mut skip = 0usize;
    let mut options_ended = false;
    for word in args {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        let text = word.text.as_str();
        if !options_ended && !word.quoted && text == "--" {
            options_ended = true;
            continue;
        }
        if !options_ended && !word.quoted && text.starts_with('-') && text.len() > 1 {
            let shape = flag_shape(program, text);
            if shape.quiet || (program == "sed" && in_place(text)) {
                return Vec::new();
            }
            pattern_given |= shape.pattern;
            target_directory_given |= shape.target_directory;
            skip = shape.skip;
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

/// What the command does with the reader's standard output: nothing
/// (`> /dev/null`), a file of its own (`> /tmp/k`), or leaves it alone.
enum Stdout {
    Untouched,
    Discarded,
    ToFile,
}

fn stdout_of(cmd: &Simple) -> Stdout {
    let mut outcome = Stdout::Untouched;
    for (op, target) in &cmd.redirects {
        if !matches!(op.as_str(), ">" | ">>" | "1>" | "1>>") || target.text.is_empty() {
            continue;
        }
        outcome = if target.text == "/dev/null" {
            Stdout::Discarded
        } else {
            Stdout::ToFile
        };
    }
    outcome
}

/// Every path a reader in the command reads, in order.
fn reads(parsed: &Parsed) -> Vec<PathRead> {
    let mut out = Vec::new();
    for cmd in parsed.judgeable() {
        // `X=$(cat f)`: the contents go to the shell, not to the tool result.
        if cmd.nested.is_some() {
            continue;
        }
        let Some(program) = cmd.program() else {
            continue;
        };
        if !READERS.contains(&program) {
            continue;
        }
        let access = match (program, stdout_of(cmd)) {
            (_, Stdout::Discarded) => continue,
            ("cp", _) | (_, Stdout::ToFile) => Access::Copy,
            (_, Stdout::Untouched) => Access::Show,
        };
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
                    access,
                });
            }
        }
    }
    out
}

fn examine(parsed: &Parsed) -> Option<Finding> {
    let (found, kind) = reads(parsed)
        .into_iter()
        .find_map(|read| secret_path(&read.shown).map(|kind| (read, kind)))?;
    let (reason, remedy) = phrase(&found.shown, kind, found.access);
    Some(Finding {
        reason,
        remedy,
        // The path operand alone: the journal must never hold the command
        // text, which may carry a pattern or a token.
        span: found.at..found.end,
    })
}

/// A finding's path resolved against the directory its clause runs in.
pub struct ResolvedRead {
    pub path: PathBuf,
    pub shown: String,
    pub access: Access,
}

/// Where a finding's path is, resolved against the directory its clause runs
/// in. `None` when the command no longer holds it or `~` has no home.
pub fn resolved(context: &Context, finding: &Finding) -> Option<ResolvedRead> {
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
    Some(ResolvedRead {
        path,
        shown: read.shown,
        access: read.access,
    })
}

fn confirm(context: &Context, finding: &Finding) -> Confirmed {
    let Some(read) = resolved(context, finding) else {
        return Confirmed::No("the command no longer matches");
    };
    let Some(kind) = secret_file(&read.path) else {
        return Confirmed::No("not there, or not a secret");
    };
    let (reason, remedy) = phrase(&read.shown, kind, read.access);
    Confirmed::YesSaying {
        floor: None,
        ceiling: Some(Stance::Advise),
        reason,
        excerpt: excerpt(kind, &read.shown),
        remedy: Some(remedy),
    }
}

/// What the journal keeps: the kind and the path, never the command.
pub fn excerpt(kind: SecretKind, shown: &str) -> String {
    format!("{} {shown}", kind.label())
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
pub fn phrase(shown: &str, kind: SecretKind, access: Access) -> (String, String) {
    let path = quoted(shown);
    let label = kind.label();
    if access == Access::Copy {
        return (
            format!(
                "`{shown}` is a {label}: copying it leaves the secret in a second file that \
                 nothing guards. The call will still run."
            ),
            format!("Use the original path ({path}) in place; do not copy the secret."),
        );
    }
    let reason = format!(
        "`{shown}` is a {label}: its contents will land in the tool result, and the \
         transcript and every later turn keep them. The call will still run."
    );
    let remedy = match kind {
        SecretKind::Netrc => format!(
            "Pass it by reference (`curl --netrc-file {path}`); check it with `wc -c {path}`."
        ),
        SecretKind::SshKey => format!("Use it through `ssh-add {path}`; never print it."),
        SecretKind::EnvFile => format!("List key names only: `cut -d= -f1 {path}`."),
        SecretKind::DirenvFile => "Do not print it: name the variable you need and read that one \
                                   from the environment."
            .to_string(),
        SecretKind::PrivateKey | SecretKind::MtlsKey => {
            format!("Pass it by reference (`--key {path}`); never print it.")
        }
        SecretKind::Npmrc => {
            format!("Ask npm instead: `npm config get registry`; as a last resort `wc -c {path}`.")
        }
        SecretKind::AwsCredentials => {
            format!("Ask the CLI instead: `aws configure list`; as a last resort `wc -c {path}`.")
        }
        SecretKind::Kubeconfig => format!(
            "Ask kubectl instead: `kubectl config view --minify`; as a last resort \
             `wc -c {path}`."
        ),
        SecretKind::DockerCredentials | SecretKind::ClaudeCredentials => format!(
            "Pass it by reference (the tool's own flag that takes a file or stdin); check it \
             with `wc -c {path}`."
        ),
    };
    (reason, remedy)
}

/// Whether the rule fires on one corpus Read case: the same name test the
/// hook's Read block makes, minus the stat and the key probe the corpus
/// cannot do.
pub fn judge_case(case: &crate::corpus::ReadCase) -> bool {
    secret_path(&case.path).is_some_and(|kind| !kind.needs_content())
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
        assert!(fires("grep -eq ~/.netrc"));
        assert!(fires("cat ~/.netrc > /tmp/k"));
        assert!(fires("grep -n -A 2 token ~/.netrc"));
        assert!(fires("awk -F : '{print $1}' ~/.netrc"));
        assert!(fires("jq --arg a b . ~/.docker/config.json"));
        assert!(fires("rg -g '*.rs' token ~/.aws/credentials"));
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
        assert!(!fires("grep -A 2 .env README.md"));
        assert!(!fires("grep -B2 .env README.md"));
        assert!(!fires("rg -g '*.rs' .env src"));
        assert!(!fires("awk -F : -v x=1 .env README.md"));
        assert!(!fires("jq --arg a .env --argjson b 1 . README.md"));
        assert!(!fires("cat ~/.netrc > /dev/null"));
        assert!(!fires("cat ~/.netrc >/dev/null 2>&1"));
        assert!(!fires("sed -i 's/a/b/' .env"));
        assert!(!fires("cp /tmp/k ~/.ssh/id_ed25519"));
        assert!(!fires("cp -t /tmp/dir /tmp/k"));
        assert!(!fires("X=$(cat ~/.netrc)"));
        assert!(!fires("cat > ~/.netrc <<'EOF'\nmachine x\nEOF\n"));
    }

    #[test]
    fn a_target_directory_flag_makes_every_operand_a_source() {
        assert!(fires("cp --target-directory=/tmp/d ~/.ssh/id_ed25519"));
        assert!(fires("cp -t/tmp/d ~/.ssh/id_ed25519"));
        assert!(fires("cp -t /tmp/d ~/.ssh/id_ed25519"));
        assert!(fires("cp --target-directory /tmp/d ~/.ssh/id_ed25519"));
        assert!(!fires("cp ~/.ssh/id_ed25519.pub /tmp/d"));
    }

    #[test]
    fn a_redirect_to_a_file_is_a_copy_and_a_pipe_is_not() {
        let access = |command: &str| {
            let parsed = lex(command);
            reads(&parsed).first().map(|read| read.access)
        };
        assert_eq!(access("cat ~/.netrc > /tmp/k"), Some(Access::Copy));
        assert_eq!(access("cat ~/.netrc >> /tmp/k"), Some(Access::Copy));
        assert_eq!(access("cat ~/.netrc | pbcopy"), Some(Access::Show));
        assert_eq!(access("cat ~/.netrc 2> /tmp/e"), Some(Access::Show));
        assert_eq!(access("cp ~/.ssh/id_rsa /tmp/k"), Some(Access::Copy));
        assert_eq!(access("cat ~/.netrc > /dev/null"), None);
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
        for (shown, kind) in [
            ("~/.ssh/id_ed25519", SecretKind::SshKey),
            ("/Users/x/.ssh/id_rsa", SecretKind::SshKey),
            ("~/.netrc", SecretKind::Netrc),
            ("~/.aws/credentials", SecretKind::AwsCredentials),
            ("~/.docker/config.json", SecretKind::DockerCredentials),
            ("~/.kube/config", SecretKind::Kubeconfig),
            ("~/.npmrc", SecretKind::Npmrc),
            ("~/.config/mtls/c.key", SecretKind::MtlsKey),
            ("~/.claude/.credentials.json", SecretKind::ClaudeCredentials),
            ("app/.env", SecretKind::EnvFile),
            ("app/.env.local", SecretKind::EnvFile),
            ("app/.envrc", SecretKind::DirenvFile),
            ("certs/server.pem", SecretKind::PrivateKey),
            ("certs/server.key", SecretKind::PrivateKey),
        ] {
            assert_eq!(secret_path(shown), Some(kind), "{shown}");
            assert!(!kind.label().is_empty());
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
        let _ = std::fs::remove_dir_all(&dir); // best-effort scratch cleanup; a leftover temp dir is harmless
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
        assert_eq!(secret_file(&key), Some(SecretKind::PrivateKey));
        assert_eq!(secret_file(&dir.join("missing.pem")), None);
        let _ = std::fs::remove_dir_all(&dir); // best-effort scratch cleanup; a leftover temp dir is harmless
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
        assert_eq!(secret_file(&ssh.join("id_*")), Some(SecretKind::SshKey));
        let _ = std::fs::remove_dir_all(&dir); // best-effort scratch cleanup; a leftover temp dir is harmless
    }

    #[test]
    fn a_directory_is_not_a_secret_file() {
        let dir = scratch("dir");
        let env = dir.join(".env");
        std::fs::create_dir_all(&env).expect("dir named .env");
        assert_eq!(secret_file(&env), None);
        let _ = std::fs::remove_dir_all(&dir); // best-effort scratch cleanup; a leftover temp dir is harmless
    }

    #[test]
    fn the_remedy_quotes_the_path_and_keeps_the_home_prefix() {
        let (reason, remedy) = phrase("/a b/it's/.env", SecretKind::EnvFile, Access::Show);
        assert!(remedy.contains(r#"'/a b/it'\''s/.env'"#), "{remedy}");
        assert!(reason.contains("will land"), "{reason}");
        assert!(
            reason.contains("contents") && !reason.contains("bytes"),
            "{reason}"
        );
        let (_, home) = phrase("~/.netrc", SecretKind::Netrc, Access::Show);
        assert!(home.contains("--netrc-file ~/.netrc"), "{home}");
        let (_, spaced) = phrase("~/my dir/.netrc", SecretKind::Netrc, Access::Show);
        assert!(spaced.contains("~/'my dir/.netrc'"), "{spaced}");
        let (copy_reason, copy_remedy) = phrase("~/.ssh/id_rsa", SecretKind::SshKey, Access::Copy);
        assert!(
            copy_reason
                .contains("copying it leaves the secret in a second file that nothing guards"),
            "{copy_reason}"
        );
        assert_eq!(
            copy_remedy,
            "Use the original path (~/.ssh/id_rsa) in place; do not copy the secret."
        );
        let (_, direnv) = phrase("app/.envrc", SecretKind::DirenvFile, Access::Show);
        assert!(direnv.contains("Do not print it"), "{direnv}");
    }

    #[test]
    fn each_kind_has_its_own_remedy_and_none_names_a_token_variable() {
        for (kind, shown, wanted) in [
            (SecretKind::Npmrc, "~/.npmrc", "npm config get registry"),
            (
                SecretKind::AwsCredentials,
                "~/.aws/credentials",
                "aws configure list",
            ),
            (
                SecretKind::Kubeconfig,
                "~/.kube/config",
                "kubectl config view --minify",
            ),
        ] {
            let (_, remedy) = phrase(shown, kind, Access::Show);
            assert!(remedy.contains(wanted), "{remedy}");
            assert!(remedy.contains(&format!("wc -c {shown}")), "{remedy}");
        }
        for kind in [
            SecretKind::SshKey,
            SecretKind::MtlsKey,
            SecretKind::ClaudeCredentials,
            SecretKind::Netrc,
            SecretKind::Npmrc,
            SecretKind::AwsCredentials,
            SecretKind::DockerCredentials,
            SecretKind::Kubeconfig,
            SecretKind::DirenvFile,
            SecretKind::EnvFile,
            SecretKind::PrivateKey,
        ] {
            let (_, remedy) = phrase("x", kind, Access::Show);
            assert!(!remedy.contains("$TOK"), "{remedy}");
        }
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
