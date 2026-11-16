//! Every name the tool says about itself, in one place (ADR-0064
//! `harness.bundle-names`, ADR-0071 `harness.renames`).
//!
//! Messages, usage text, error prefixes, `--version` lines, command hints and
//! the hook decision prefix are built from these, never from a literal: a
//! later name is one change here. The names test (`tests/names.rs`) fails on a
//! literal `amont-agent` in any other Rust string.
//!
//! In a format string, name the constant inline: `eprintln!("{BIN}: …")`.
//! For a `&'static str` built at compile time, use the [`bin!`] macro with
//! `concat!`.

/// The binary's name as a literal, for `concat!` in a `&'static str`.
#[doc(hidden)]
#[macro_export]
macro_rules! bin {
    () => {
        "greenbelay"
    };
}

/// The product's name, as it introduces itself.
pub const PRODUCT: &str = bin!();

/// The binary.
pub const BIN: &str = bin!();

/// The binary's old name, recognised in hook entries and commands until the
/// census; no longer installed (ADR-0071).
pub const ALIAS: &str = "amont-agent";

/// The state and config directory segment (`~/.claude/greenbelay/`) and the
/// plugin marketplace id.
pub const DATA_DIR: &str = "greenbelay";

/// The `tool` field value of the records other programs read.
pub const RECORD_TOOL: &str = "greenbelay";

/// The header line of a journal file.
pub const JOURNAL_HEADER: &str = "greenbelay-v1";

/// The header line of a corpus (`.cases`) file.
pub const CORPUS_HEADER: &str = "# greenbelay-cases-v1";

/// The schema of a review-outcome row.
pub const REVIEW_OUTCOME_SCHEMA: &str = "greenbelay-review-outcome-v1";

/// The schema of a review-merge row.
pub const REVIEW_MERGE_SCHEMA: &str = "greenbelay-review-merge-v1";

/// The schema of a review-disposition row.
pub const REVIEW_DISPOSITION_SCHEMA: &str = "greenbelay-review-disposition-v1";

/// The schema of an implementation-review outcome row.
pub const IMPL_REVIEW_OUTCOME_SCHEMA: &str = "greenbelay-impl-review-outcome-v1";

/// The word naming this tool on the `producer` line of a provenance record.
pub const PROVENANCE_PRODUCER: &str = "greenbelay";

/// The prefix of an atomic write's temporary file.
pub const TEMP_PREFIX: &str = ".greenbelay-tmp-";

/// The lock file of the one-time migration, directly in the config directory
/// (never inside a root that moves).
pub const MIGRATE_LOCK_FILE: &str = "greenbelay-migrate.lock";

/// Written when the migration finished; its presence short-circuits every hook.
pub const MIGRATE_DONE_MARKER: &str = "greenbelay-migrated";

/// Written by the reverse: a hook never migrates forward while it exists.
pub const MIGRATE_REVERSED_MARKER: &str = "greenbelay-migrate-reversed";

/// Written when a step failed, with the step and what to fix.
pub const MIGRATE_FAILED_MARKER: &str = "greenbelay-migrate-failed";

/// The git config section every key this tool reads or writes lives under.
pub const CONFIG_PREFIX: &str = "greenbelay.";

/// The prefix of every environment variable this tool reads.
pub const ENV_PREFIX: &str = "GREENBELAY_";

/// A git config key as a `&'static str`: `config_key!("stance")` is
/// `greenbelay.stance`.
#[doc(hidden)]
#[macro_export]
macro_rules! config_key {
    ($name:literal) => {
        concat!("greenbelay.", $name)
    };
}

/// An environment variable as a `&'static str`: `env_var!("OFF")` is
/// `GREENBELAY_OFF`.
#[doc(hidden)]
#[macro_export]
macro_rules! env_var {
    ($name:literal) => {
        concat!("GREENBELAY_", $name)
    };
}

/// A `git config --get-regexp` pattern for the keys under [`CONFIG_PREFIX`]
/// whose remainder matches `rest` (a regular expression, possibly empty).
pub fn config_pattern(rest: &str) -> String {
    format!("^{}{rest}", CONFIG_PREFIX.replace('.', "\\."))
}

/// The names the tool had before it was renamed. Read by the readers (old
/// rows and committed corpus files stay readable) and by the later migration;
/// removed at the programme census.
pub mod old {
    /// The old git config prefix.
    pub const CONFIG_PREFIX: &str = "amont.agent.";

    /// The old environment-variable prefix.
    pub const ENV_PREFIX: &str = "AMONT_AGENT_";

    /// The old state and config directory segment and marketplace id.
    pub const DATA_DIR: &str = "amont-agent";

    /// The old `tool` field value of records.
    pub const RECORD_TOOL: &str = "amont-agent";

    /// The old journal header.
    pub const JOURNAL_HEADER: &str = "amont-agent-v1";

    /// The old corpus header.
    pub const CORPUS_HEADER: &str = "# amont-agent-cases-v1";

    /// The old review-outcome schema.
    pub const REVIEW_OUTCOME_SCHEMA: &str = "amont-agent-review-outcome-v1";

    /// The old review-merge schema.
    pub const REVIEW_MERGE_SCHEMA: &str = "amont-agent-review-merge-v1";

    /// The old review-disposition schema.
    pub const REVIEW_DISPOSITION_SCHEMA: &str = "amont-agent-review-disposition-v1";

    /// The old implementation-review outcome schema.
    pub const IMPL_REVIEW_OUTCOME_SCHEMA: &str = "amont-agent-impl-review-outcome-v1";

    /// The old provenance producer word.
    pub const PROVENANCE_PRODUCER: &str = "amont-agent";

    /// The old atomic temp-file prefix.
    pub const TEMP_PREFIX: &str = ".amont-agent-tmp-";
}

fn strip_exe(name: &str) -> &str {
    let split = name.len().saturating_sub(4);
    match name.get(split..) {
        Some(tail) if tail.eq_ignore_ascii_case(".exe") => &name[..split],
        _ => name,
    }
}

/// Whether `program` (a command word's program, with or without a directory
/// and a trailing `.exe`) is this tool under its name or its alias.
pub fn is_ours_program(program: &str) -> bool {
    program_name(program).eq_ignore_ascii_case(BIN) || is_alias_program(program)
}

/// Whether `program` is this tool under its old name.
pub fn is_alias_program(program: &str) -> bool {
    program_name(program).eq_ignore_ascii_case(ALIAS)
}

fn program_name(program: &str) -> &str {
    strip_exe(program.rsplit(['/', '\\']).next().unwrap_or(program))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_names_are_ours_with_a_directory_or_exe() {
        for program in [
            "greenbelay",
            "amont-agent",
            "/usr/local/bin/greenbelay",
            "/usr/local/bin/amont-agent",
            "greenbelay.exe",
            "C:\\bin\\amont-agent.exe",
            "GREENBELAY.EXE",
            "C:\\bin\\Amont-Agent.Exe",
        ] {
            assert!(is_ours_program(program), "{program}");
        }
    }

    #[test]
    fn only_the_old_name_is_the_alias() {
        assert!(is_alias_program("/bin/amont-agent"));
        assert!(is_alias_program("C:\\bin\\Amont-Agent.Exe"));
        assert!(!is_alias_program("greenbelay"));
    }

    #[test]
    fn other_programs_are_not_ours() {
        for program in [
            "",
            "git",
            "amont",
            "greenbelay-fleet",
            "my-greenbelay",
            "amont-agent2",
        ] {
            assert!(!is_ours_program(program), "{program}");
        }
    }

    #[test]
    fn the_name_macros_expand_to_the_prefixes() {
        assert_eq!(config_key!("x"), format!("{CONFIG_PREFIX}x"));
        assert_eq!(env_var!("X"), format!("{ENV_PREFIX}X"));
    }

    #[test]
    fn old_names_are_the_previous_literals() {
        assert_eq!(old::DATA_DIR, "amont-agent");
        assert_eq!(old::RECORD_TOOL, "amont-agent");
        assert_eq!(old::JOURNAL_HEADER, "amont-agent-v1");
        assert_eq!(old::CORPUS_HEADER, "# amont-agent-cases-v1");
        assert_eq!(old::REVIEW_OUTCOME_SCHEMA, "amont-agent-review-outcome-v1");
        assert_eq!(old::REVIEW_MERGE_SCHEMA, "amont-agent-review-merge-v1");
        assert_eq!(
            old::REVIEW_DISPOSITION_SCHEMA,
            "amont-agent-review-disposition-v1"
        );
        assert_eq!(
            old::IMPL_REVIEW_OUTCOME_SCHEMA,
            "amont-agent-impl-review-outcome-v1"
        );
        assert_eq!(old::PROVENANCE_PRODUCER, "amont-agent");
        assert_eq!(old::TEMP_PREFIX, ".amont-agent-tmp-");
    }
}
