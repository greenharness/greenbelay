//! The tool's name is said from `names` alone (ADR-0064, ADR-0071). This test fails on `amont-agent` inside any Rust
//! string literal under `crates/*/src`, `src` and `tests`, except in a
//! `names.rs`, except the alias binary's file name.

use std::path::{Path, PathBuf};

const NAME: &str = "amont-agent";

/// Exact lines, by file, that deliberately plant an old form.
const KEPT_LINES: &[(&str, &str)] = &[];

/// The old environment-variable and git-config prefixes, which only
/// `names::old` and this test may spell.
const OLD_PREFIXES: &[&str] = &["amont.agent.", "AMONT_AGENT_"];

/// The module that says the names, and this test, which spells them out.
const NAMES_MODULES: &[&str] = &["crates/greenbelay-guard/src/names.rs", "tests/names.rs"];

/// A string literal: where its text starts in the source, and the text
/// between the quotes as written (escapes and all).
struct Literal {
    text_start: usize,
    text: String,
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Every string literal of a Rust source, comments skipped.
fn literals(source: &str) -> Vec<Literal> {
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let rest = &source[at..];
        if rest.starts_with("//") {
            at += rest.find('\n').unwrap_or(rest.len());
        } else if rest.starts_with("/*") {
            let mut depth = 0;
            while at < bytes.len() {
                if source[at..].starts_with("/*") {
                    depth += 1;
                    at += 2;
                } else if source[at..].starts_with("*/") {
                    depth -= 1;
                    at += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    at += 1;
                }
            }
        } else if let Some(open) = raw_string_open(source, at) {
            let hash_count = source[at..open].bytes().filter(|b| *b == b'#').count();
            let close = format!("\"{}", "#".repeat(hash_count));
            let end = source[open..]
                .find(&close)
                .map_or(bytes.len(), |i| open + i);
            found.push(Literal {
                text_start: open,
                text: source[open..end].to_string(),
            });
            at = end + close.len();
        } else if bytes[at] == b'"' {
            let start = at + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            end = end.min(bytes.len());
            found.push(Literal {
                text_start: start,
                text: source[start..end].to_string(),
            });
            at = end + 1;
        } else if bytes[at] == b'\'' {
            // A char literal, or a lifetime.
            if bytes.get(at + 1) == Some(&b'\\') {
                at += source
                    .get(at + 3..)
                    .and_then(|tail| tail.find('\''))
                    .map_or(1, |i| i + 4);
            } else if bytes.get(at + 2) == Some(&b'\'') {
                at += 3;
            } else {
                at += 1;
            }
        } else {
            at += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    found
}

/// The offset just past the opening quote of a raw string starting at `at`
/// (`r"`, `r#"`, `br#"`), if one starts there.
fn raw_string_open(source: &str, at: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if at > 0 && is_word_byte(bytes[at - 1]) {
        return None;
    }
    let mut cursor = at;
    if bytes.get(cursor) == Some(&b'b') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'r') {
        return None;
    }
    cursor += 1;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    (bytes.get(cursor) == Some(&b'"')).then_some(cursor + 1)
}

/// Whether one occurrence of the name, at `at` in `literal`, is not the tool
/// saying its own name: part of a longer word, or the alias binary's file name.
fn is_kept_data_name(source: &str, literal: &Literal, at: usize) -> bool {
    let text = literal.text.as_str();
    let before = &text[..at];
    let line_start = source[..literal.text_start]
        .rfind('\n')
        .map_or(0, |i| i + 1);
    let before_on_line = &source[line_start..literal.text_start];

    // The word is longer than the name: `CARGO_BIN_EXE_greenbelay`.
    if before.bytes().last().is_some_and(is_word_byte) {
        return true;
    }
    // A literal that is exactly the name, joined onto a directory: the alias
    // binary's file name (`ALIAS`), which tests copy and look for.
    text == NAME
        && before_on_line
            .trim_end_matches(['"', 'r', '#', ' '])
            .ends_with(".join(")
}

/// The line of every occurrence of the name in a string literal of `source`
/// that is not a kept data name.
fn stray_names(source: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    for literal in literals(source) {
        let mut from = 0;
        while let Some(found) = literal.text[from..].find(NAME) {
            let at = from + found;
            from = at + NAME.len();
            let after = literal.text.as_bytes().get(from).copied();
            if after.is_some_and(is_word_byte) {
                continue;
            }
            if !is_kept_data_name(source, &literal, at) {
                lines.push(source[..literal.text_start + at].matches('\n').count() + 1);
            }
        }
    }
    lines
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}

#[test]
fn no_source_string_names_the_tool_except_the_names_module_and_kept_data_names() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = rust_files(&root.join("src"));
    files.extend(rust_files(&root.join("tests")));
    for krate in std::fs::read_dir(root.join("crates"))
        .expect("crates dir")
        .flatten()
    {
        files.extend(rust_files(&krate.path().join("src")));
    }
    assert!(files.len() > 50, "found only {} sources", files.len());
    let mut strays = Vec::new();
    for file in files {
        // Forward slashes on every platform: NAMES_MODULES and KEPT_LINES
        // name paths that way, and Windows would display `crates\...`.
        let shown = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string()
            .replace('\\', "/");
        if NAMES_MODULES.contains(&shown.as_str()) {
            continue;
        }
        let source = std::fs::read_to_string(&file).expect("readable source");
        for line in stray_names(&source) {
            let text = source.lines().nth(line - 1).unwrap_or_default().trim();
            if KEPT_LINES
                .iter()
                .any(|(path, kept)| *path == shown && *kept == text)
            {
                continue;
            }
            strays.push(format!("{shown}:{line}"));
        }
    }
    assert!(
        strays.is_empty(),
        "`amont-agent` in a string literal; say it through `names` (or, for a data name \
         a test that plants an old form goes in the kept list in tests/names.rs):\n{}",
        strays.join("\n")
    );
}

#[test]
fn a_planted_message_is_found_and_the_alias_file_name_is_not() {
    assert_eq!(
        stray_names("fn f() { eprintln!(\"amont-agent: x\"); }"),
        vec![1]
    );
    assert_eq!(
        stray_names("const U: &str = r#\"usage: amont-agent open\"#;"),
        vec![1]
    );
    assert_eq!(stray_names("let a = \"\n\namont-agent/rule: x\";"), vec![3]);
    assert_eq!(
        stray_names("// amont-agent: a comment\nlet a = 1;"),
        Vec::<usize>::new()
    );
    for kept in [
        "let a = dir.join(\"amont-agent\");",
        "let e = env!(\"CARGO_BIN_EXE_greenbelay\");",
    ] {
        assert_eq!(stray_names(kept), Vec::<usize>::new(), "{kept}");
    }
}

/// Whether any string literal of `source` contains an old prefix, in any
/// case: git folds a key's section to lower case, so `AMONT.agent.x` is the
/// old name too.
fn has_old_prefix_literal(source: &str) -> bool {
    literals(source).iter().any(|literal| {
        let text = literal.text.to_ascii_lowercase();
        OLD_PREFIXES
            .iter()
            .any(|old| text.contains(&old.to_ascii_lowercase()))
    })
}

#[test]
fn no_source_string_spells_an_old_env_or_config_prefix() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = rust_files(&root.join("src"));
    files.extend(rust_files(&root.join("tests")));
    for krate in std::fs::read_dir(root.join("crates"))
        .expect("crates dir")
        .flatten()
    {
        files.extend(rust_files(&krate.path().join("src")));
    }
    assert!(files.len() > 50, "found only {} sources", files.len());
    let mut strays = Vec::new();
    for file in files {
        let shown = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string()
            .replace('\\', "/");
        if NAMES_MODULES.contains(&shown.as_str()) {
            continue;
        }
        let source = std::fs::read_to_string(&file).expect("readable source");
        if has_old_prefix_literal(&source) {
            strays.push(shown);
        }
    }
    assert!(
        strays.is_empty(),
        "an old `amont.agent.` or `AMONT_AGENT_` name in a string literal; the old prefixes live \
         in `names::old` alone:\n{}",
        strays.join("\n")
    );
}

#[test]
fn a_planted_old_prefix_literal_is_found() {
    assert!(has_old_prefix_literal("let k = \"amont.agent.stance\";"));
    assert!(has_old_prefix_literal("std::env::var(\"AMONT_AGENT_OFF\")"));
    assert!(has_old_prefix_literal("let k = \"AMONT.agent.x.STANCE\";"));
    assert!(!has_old_prefix_literal(
        "// amont.agent.stance in a comment\nlet k = \"greenbelay.stance\";"
    ));
}
