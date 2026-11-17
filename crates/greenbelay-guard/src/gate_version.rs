//! What version a gate binary says it is: `<program> --version`, read once
//! under a short deadline, and compared as a plain triple.
//!
//! Shared by the rules that speak only when the gate on `PATH` is new enough
//! to have what they would recommend (`heavy-build-unqueued`,
//! `land-detached`).

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// `major.minor.patch`; tuples compare in that order.
pub type Version = (u64, u64, u64);

/// How long `<program> --version` may take.
const PROBE_DEADLINE: Duration = Duration::from_millis(1500);

/// `<program> --version`, killed at [`PROBE_DEADLINE`]; `None` when the
/// program is missing, fails, hangs or prints no version.
pub fn probe(program: &str) -> Option<Version> {
    let mut child = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + PROBE_DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill(); // past the deadline the answer is already unknown
                let _ = child.wait(); // a reap that fails leaves nothing more to report
                return None;
            }
        }
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text).ok()?;
    parse_version(&text)
}

/// The first `major.minor.patch` in `--version` output such as `amont 1.62.0`.
pub fn parse_version(text: &str) -> Option<Version> {
    text.split_whitespace().find_map(|token| {
        let mut parts = token.trim_start_matches('v').split('.').map(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u64>().ok()
        });
        Some((parts.next()??, parts.next()??, parts.next()??))
    })
}

/// Pure: whether `found` is at least `minimum`. A caller whose version could
/// not be read decides what that means before it asks.
pub fn at_least(found: Version, minimum: Version) -> bool {
    found >= minimum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_read_from_the_output() {
        assert_eq!(parse_version("amont 1.62.0\n"), Some((1, 62, 0)));
        assert_eq!(parse_version("amont v1.61.1-rc1"), Some((1, 61, 1)));
        assert_eq!(parse_version("greengate 4.9.0"), Some((4, 9, 0)));
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn at_least_compares_numerically_not_textually() {
        assert!(at_least((4, 9, 0), (4, 9, 0)));
        assert!(at_least((4, 10, 0), (4, 9, 0)));
        assert!(at_least((5, 0, 0), (4, 9, 3)));
        assert!(!at_least((4, 8, 9), (4, 9, 0)));
        assert!(!at_least((3, 99, 99), (4, 0, 0)));
    }

    #[test]
    fn a_program_that_does_not_exist_has_no_version() {
        assert_eq!(probe("greenbelay-test-no-such-gate"), None);
    }
}
