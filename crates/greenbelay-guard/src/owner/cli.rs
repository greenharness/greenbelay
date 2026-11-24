//! `greenbelay owner claim|release|show`: the command line over the owner store.

use super::{
    claim_ticket, claim_worktree, read_ticket, read_worktree, release, Claim, Contest, Error,
    Identity, Outcome, Owner, Reading, Released, Slot, State, Store, TicketKey,
};
use crate::host_session;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const EXIT_OK: u8 = 0;
pub const EXIT_FAILED: u8 = 1;
pub const EXIT_USAGE: u8 = 2;
pub const EXIT_HELD: u8 = 3;

pub const HELP: &str = concat!(
    "usage: ",
    crate::bin!(),
    " owner <command>

  owner claim [--ticket <repo>/<prog>/<id>] [--name <n>] [--ref <r>] [--steal]
                        claim the current worktree, and the ticket when given;
                        --name and --ref say how to message this session
  owner release [--ticket <repo>/<prog>/<id>]
                        release what this session holds
  owner show [--worktree <p> | --ticket <repo>/<prog>/<id>] [--json]
                        print who owns the worktree (default: the current one)
                        or the ticket, and whether that owner is still live

exit status:
  0  claimed, released (or already free), or an owner shown
  1  no owner recorded (show), or any other failure
  2  usage error
  3  another session holds it, live or of unknown liveness, or its record
     does not read (claim, release); claim --steal takes over

claim takes the worktree, then the ticket: an exit 3 on the ticket leaves
the worktree claimed. What was done is said on stderr.
"
);

enum Target {
    Worktree(PathBuf),
    Ticket(TicketKey),
}

impl Target {
    fn word(&self) -> &'static str {
        match self {
            Target::Worktree(_) => "worktree",
            Target::Ticket(_) => "ticket",
        }
    }

    fn shown(&self) -> String {
        match self {
            Target::Worktree(path) => path.display().to_string(),
            Target::Ticket(key) => format!("{}/{}/{}", key.repo, key.programme, key.ticket),
        }
    }
}

#[derive(Default)]
struct Options {
    ticket: Option<String>,
    worktree: Option<String>,
    name: Option<String>,
    reference: Option<String>,
    steal: bool,
    json: bool,
    help: bool,
}

fn parse(args: &[String], allowed: &[&str]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        index += 1;
        if arg == "--help" || arg == "-h" {
            options.help = true;
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (arg, None),
        };
        if !allowed.contains(&flag) {
            return Err(format!("unknown argument `{arg}`"));
        }
        if matches!(flag, "--steal" | "--json") {
            if inline.is_some() {
                return Err(format!("`{flag}` takes no value"));
            }
            if flag == "--steal" {
                options.steal = true;
            } else {
                options.json = true;
            }
            continue;
        }
        let value = match inline {
            Some(value) => value,
            None => {
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{flag}` needs a value"))?;
                index += 1;
                value.clone()
            }
        };
        match flag {
            "--ticket" => options.ticket = Some(value),
            "--worktree" => options.worktree = Some(value),
            "--name" => options.name = Some(value),
            _ => options.reference = Some(value),
        }
    }
    Ok(options)
}

fn ticket_key(text: &str) -> Result<TicketKey, String> {
    let parts: Vec<&str> = text.split('/').collect();
    let [repo, programme, ticket] = parts.as_slice() else {
        return Err(format!(
            "`{text}` is not <repo>/<prog>/<id>: three segments separated by /"
        ));
    };
    TicketKey::new(repo, programme, ticket).map_err(|failure: Error| failure.to_string())
}

fn usage(message: &str) -> u8 {
    eprintln!("{}: owner: {message}\n{HELP}", crate::bin!());
    EXIT_USAGE
}

fn fail(message: &str) -> u8 {
    eprintln!("{}: owner: {message}", crate::bin!());
    EXIT_FAILED
}

fn seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn worktree_root(from: &Path) -> Option<PathBuf> {
    crate::activity::resolve(from).map(|place| place.worktree_root)
}

/// The exit status of `owner <args>`; output goes to stdout and stderr.
pub fn run(args: &[String]) -> u8 {
    let Some((sub, rest)) = args.split_first() else {
        return usage("no command given");
    };
    match sub.as_str() {
        "--help" | "-h" | "help" => {
            print!("{HELP}");
            EXIT_OK
        }
        "claim" => with_options(rest, &["--ticket", "--name", "--ref", "--steal"], claim),
        "release" => with_options(rest, &["--ticket"], release_command),
        "show" => with_options(rest, &["--worktree", "--ticket", "--json"], show),
        other => usage(&format!("unknown command `{other}`")),
    }
}

fn with_options(args: &[String], allowed: &[&str], run: fn(Options) -> u8) -> u8 {
    match parse(args, allowed) {
        Err(message) => usage(&message),
        Ok(options) if options.help => {
            print!("{HELP}");
            EXIT_OK
        }
        Ok(options) => run(options),
    }
}

fn targets(options: &Options) -> Result<Vec<Target>, u8> {
    let mut targets = Vec::new();
    let cwd = std::env::current_dir().ok();
    let worktree = cwd.as_deref().and_then(worktree_root);
    match (&options.ticket, worktree) {
        (_, Some(root)) => targets.push(Target::Worktree(root)),
        (Some(_), None) => {}
        (None, None) => return Err(fail("not inside a git worktree; name a --ticket")),
    }
    if let Some(text) = &options.ticket {
        targets.push(Target::Ticket(ticket_key(text).map_err(|why| usage(&why))?));
    }
    Ok(targets)
}

fn describe_holder(owner: &Owner) -> String {
    owner.holder()
}

/// The `owner claim` that takes `target`, with `extra` flags.
fn claim_command(target: &Target, extra: &str) -> String {
    match target {
        Target::Worktree(_) => format!("{} owner claim{extra}", crate::bin!()),
        Target::Ticket(_) => format!(
            "{} owner claim --ticket {}{extra}",
            crate::bin!(),
            target.shown()
        ),
    }
}

fn refuse(target: &Target, owner: &Owner, state: &State) -> u8 {
    let holder = describe_holder(owner);
    let place = format!("{} {}", target.word(), target.shown());
    match state {
        State::Live | State::Unknown => {
            let how = if matches!(state, State::Live) {
                "live"
            } else {
                "liveness unknown (host registry unreadable)"
            };
            let message = match owner.identity.name.as_str() {
                "" => String::new(),
                name => format!(" Message it: SendMessage to \"{name}\"."),
            };
            eprintln!(
                "refused: {holder} holds {place}, {how}.{message} To take over anyway: {}",
                claim_command(target, " --steal"),
            );
        }
        State::Cleared { .. } | State::Gone | State::Stale => {
            eprintln!(
                "refused: you do not hold {place}; its owner {holder} is {}, so it is free: {}",
                state.word(),
                claim_command(target, ""),
            );
        }
    }
    EXIT_HELD
}

fn claim(options: Options) -> u8 {
    let targets = match targets(&options) {
        Ok(targets) => targets,
        Err(code) => return code,
    };
    let Some(store) = Store::ambient() else {
        return fail("no data directory or host registry directory");
    };
    let Some(host) = host_session::caller() else {
        return fail(
            "cannot tell which session this is: no Claude Code session above this process",
        );
    };
    let contest = if options.steal {
        Contest::Steal
    } else {
        Contest::Respect
    };
    let now = seconds_now();
    let place = std::env::current_dir()
        .ok()
        .and_then(|cwd| crate::activity::resolve(&cwd));
    for target in &targets {
        let before = match target {
            Target::Worktree(path) => read_worktree(&store, path, now),
            Target::Ticket(key) => read_ticket(&store, key, now),
        };
        let reference = options.reference.clone().or(match before {
            Reading::Held { owner, .. } if owner.identity.session == host.session => {
                owner.identity.reference
            }
            _ => None,
        });
        let claim = Claim {
            repo_identity: place
                .as_ref()
                .map(|place| place.repo_identity.clone())
                .unwrap_or_default(),
            branch: place
                .as_ref()
                .map_or_else(|| "-".to_owned(), |place| place.branch.clone()),
            identity: Identity {
                session: host.session.clone(),
                pid: host.pid,
                started_at: host.started_at,
                name: options.name.clone().unwrap_or_else(|| host.name.clone()),
                reference,
            },
        };
        let outcome = match target {
            Target::Worktree(path) => claim_worktree(&store, path, &claim, contest, now),
            Target::Ticket(key) => {
                let map_dir = place.as_ref().map_or_else(PathBuf::new, |place| {
                    place
                        .worktree_root
                        .join("docs/programmes")
                        .join(&key.programme)
                });
                claim_ticket(&store, key, &map_dir, &claim, contest, now)
            }
        };
        match outcome {
            Ok(Outcome::Claimed | Outcome::Refreshed) => {
                eprintln!("claimed {} {}", target.word(), target.shown());
            }
            Ok(Outcome::Unreadable { path }) => {
                eprintln!(
                    "refused: the record for {} {} does not read ({}); to replace it: {}",
                    target.word(),
                    target.shown(),
                    path.display(),
                    claim_command(target, " --steal"),
                );
                return EXIT_HELD;
            }
            Ok(Outcome::TakenOver { from }) => {
                eprintln!(
                    "claimed {} {} (took over from {})",
                    target.word(),
                    target.shown(),
                    from.name
                );
            }
            Ok(Outcome::HeldBy { owner, state }) => return refuse(target, &owner, &state),
            Ok(Outcome::LockBusy) => {
                return fail(&format!(
                    "{} is being claimed by another process; retry",
                    target.word()
                ));
            }
            Err(failure) => return fail(&failure.to_string()),
        }
    }
    EXIT_OK
}

fn release_command(options: Options) -> u8 {
    let targets = match targets(&options) {
        Ok(targets) => targets,
        Err(code) => return code,
    };
    let Some(store) = Store::ambient() else {
        return fail("no data directory or host registry directory");
    };
    let Some(host) = host_session::caller() else {
        return fail(
            "cannot tell which session this is: no Claude Code session above this process",
        );
    };
    let now = seconds_now();
    for target in &targets {
        let slot = match target {
            Target::Worktree(path) => Slot::Worktree(path.clone()),
            Target::Ticket(key) => Slot::Ticket(key.clone()),
        };
        match release(&store, &slot, &host.session, now) {
            Ok(Released::Released) => {
                eprintln!("released {} {}", target.word(), target.shown());
            }
            Ok(Released::AlreadyFree) => {
                eprintln!("already free: {} {}", target.word(), target.shown());
            }
            Ok(Released::HeldBy { owner, state }) => return refuse(target, &owner, &state),
            Ok(Released::LockBusy) => {
                return fail(&format!(
                    "{} is being changed by another process; retry",
                    target.word()
                ));
            }
            Err(failure) => return fail(&failure.to_string()),
        }
    }
    EXIT_OK
}

fn show(options: Options) -> u8 {
    if options.ticket.is_some() && options.worktree.is_some() {
        return usage("--worktree and --ticket exclude each other");
    }
    let Some(store) = Store::ambient() else {
        return fail("no data directory or host registry directory");
    };
    let now = seconds_now();
    let (target, shown_arg, reading) = match &options.ticket {
        Some(text) => {
            let key = match ticket_key(text) {
                Ok(key) => key,
                Err(why) => return usage(&why),
            };
            let reading = read_ticket(&store, &key, now);
            (Target::Ticket(key), text.clone(), reading)
        }
        None => {
            let given = match &options.worktree {
                Some(path) => PathBuf::from(path),
                None => match std::env::current_dir() {
                    Ok(cwd) => cwd,
                    Err(failure) => return fail(&failure.to_string()),
                },
            };
            let root = worktree_root(&given).unwrap_or_else(|| given.clone());
            let reading = read_worktree(&store, &root, now);
            (Target::Worktree(root), given.display().to_string(), reading)
        }
    };
    match reading {
        Reading::Free => {
            eprintln!("no owner recorded for {shown_arg}");
            EXIT_FAILED
        }
        Reading::Unknown => {
            if options.json {
                println!(
                    "{}",
                    json!({
                        "place": target.word(),
                        "target": target.shown(),
                        "session": null,
                        "name": null,
                        "ref": null,
                        "pid": null,
                        "started_at": null,
                        "branch": null,
                        "claimed_at": null,
                        "refreshed_at": null,
                        "state": "unknown",
                    })
                );
            } else {
                println!("owner: unknown\nstate: unknown: the record is unreadable");
            }
            EXIT_OK
        }
        Reading::Held { owner, state } => {
            print_owner(&target, &owner, &state, now, options.json);
            EXIT_OK
        }
    }
}

fn print_owner(target: &Target, owner: &Owner, state: &State, now: u64, as_json: bool) {
    let identity = &owner.identity;
    if as_json {
        let word = state.word();
        let now_session = match state {
            State::Cleared { now, .. } => Some(now.as_str()),
            _ => None,
        };
        let mut value = json!({
            "place": target.word(),
            "target": target.shown(),
            "session": identity.session,
            "name": identity.name,
            "ref": identity.reference,
            "pid": identity.pid,
            "started_at": identity.started_at,
            "branch": owner.branch,
            "claimed_at": owner.claimed_at,
            "refreshed_at": owner.refreshed_at,
            "state": word,
        });
        if let (Some(now_session), Value::Object(map)) = (now_session, &mut value) {
            map.insert("now_session".into(), json!(now_session));
        }
        println!("{value}");
        return;
    }
    let ago = crate::activity::ago(
        UNIX_EPOCH + Duration::from_secs(now),
        UNIX_EPOCH + Duration::from_secs(owner.claimed_at),
    );
    println!(
        "owner: {} / {}, claimed {ago} ago",
        describe_holder(owner),
        identity.session
    );
    println!("state: {}", state.describe());
}

pub const WHO_PUSHED_HELP: &str = concat!(
    "usage: ",
    crate::bin!(),
    " who-pushed <branch|sha> [--repo <r>] [--json]

Each recorded push of a branch, or of a commit (a sha prefix of at least 7
characters), newest first, one line each:
  <ago>  <name>  <uuid8>  <state>  <branch>  <sha7>
--repo keeps the pushes of the repository named <r>; --json prints one array
with the full session UUID. A push is recorded only when it left the machine
(a ref-update line, or a land receipt that says landed).

exit status:
  0  pushes shown
  1  no push recorded for it, or any other failure
  2  usage error
"
);

/// The exit status of `who-pushed <args>`.
pub fn who_pushed(args: &[String]) -> u8 {
    let mut target: Option<&str> = None;
    let mut repo: Option<&str> = None;
    let mut as_json = false;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--help" | "-h" => {
                print!("{WHO_PUSHED_HELP}");
                return EXIT_OK;
            }
            "--json" => as_json = true,
            "--repo" => match words.next() {
                Some(name) => repo = Some(name),
                None => return who_usage("--repo needs a repository name"),
            },
            flag if flag.starts_with("--repo=") => repo = flag.strip_prefix("--repo="),
            flag if flag.starts_with('-') => return who_usage(&format!("unknown option `{flag}`")),
            operand if target.is_none() => target = Some(operand),
            extra => return who_usage(&format!("one branch or sha, not also `{extra}`")),
        }
    }
    let Some(target) = target else {
        return who_usage("name a branch or a sha");
    };
    let Some(store) = Store::ambient() else {
        eprintln!("{}: who-pushed: no data directory", crate::bin!());
        return EXIT_FAILED;
    };
    let now = seconds_now();
    let is_sha = target.len() >= 7 && target.chars().all(|c| c.is_ascii_hexdigit());
    let found: Vec<super::pushes::Pushed> = super::pushes::read_all(&store)
        .into_iter()
        .rev()
        .filter(|push| push.branch == target || (is_sha && push.sha.starts_with(target)))
        .filter(|push| {
            repo.is_none_or(|name| crate::activity::repo_name_of(&push.repo_identity) == name)
        })
        .collect();
    if found.is_empty() {
        if as_json {
            println!("[]");
        }
        eprintln!("no pushes recorded for {target} (the log keeps about the last 2 MB of pushes)");
        return EXIT_FAILED;
    }
    let state_of = |push: &super::pushes::Pushed| -> &'static str {
        match host_session::liveness_in(
            &store.registry,
            &host_session::Recorded {
                uuid: push.identity.session.clone(),
                pid: push.identity.pid,
                started_at: push.identity.started_at,
            },
        ) {
            host_session::Liveness::Live => "live",
            host_session::Liveness::Cleared { .. } => "cleared",
            host_session::Liveness::Gone => "gone",
            host_session::Liveness::Unknown => "unknown",
        }
    };
    if as_json {
        let rows: Vec<Value> = found
            .iter()
            .map(|push| {
                json!({
                    "at": push.at,
                    "session": push.identity.session,
                    "name": push.identity.name,
                    "pid": push.identity.pid,
                    "state": state_of(push),
                    "repo": crate::activity::repo_name_of(&push.repo_identity),
                    "branch": push.branch,
                    "sha": push.sha,
                })
            })
            .collect();
        println!("{}", Value::Array(rows));
        return EXIT_OK;
    }
    for push in &found {
        let ago = crate::activity::ago(
            UNIX_EPOCH + Duration::from_secs(now),
            UNIX_EPOCH + Duration::from_secs(push.at),
        );
        let name = if push.identity.name.is_empty() {
            "-"
        } else {
            push.identity.name.as_str()
        };
        println!(
            "{ago} ago  {name}  {}  {}  {}  {}",
            push.identity
                .session
                .get(..8)
                .unwrap_or(&push.identity.session),
            state_of(push),
            push.branch,
            push.sha.get(..7).unwrap_or(&push.sha),
        );
    }
    EXIT_OK
}

fn who_usage(message: &str) -> u8 {
    eprintln!(
        "{}: who-pushed: {message}\n{WHO_PUSHED_HELP}",
        crate::bin!()
    );
    EXIT_USAGE
}
