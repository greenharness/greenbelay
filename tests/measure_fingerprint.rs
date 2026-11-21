//! `tools/measure_fingerprint.py` and its first caller, `tools/read-rate.py`,
//! run with `python3 -I` as the fleet runs them: a sibling `import` fails here
//! as it would there. Unix only, as the helper is: it locks with `fcntl`.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const HELPER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/measure_fingerprint.py");
const READ_RATE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/read-rate.py");

fn python(args: &[&str]) -> Output {
    Command::new("python3")
        .arg("-I")
        .args(args)
        .output()
        .expect("python3 runs")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("measure-fp-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir); // absent on a first run; create_dir_all below is the check
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A stamp built by the helper's own `stamp()`; `window` is the only recipe
/// difference between callers.
fn make_stamp(dir: &Path, name: &str, window: &str) -> String {
    let path = dir.join(name);
    let code = r#"
import importlib.util, sys
spec = importlib.util.spec_from_file_location("mf", sys.argv[1])
mf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mf)
r = mf.recipe(sys.argv[1], "python3.x", "src", "schema", {"n": 1}, sys.argv[3])
mf.stamp(sys.argv[2], r, {"files": 1})
"#;
    let out = python(&["-c", code, HELPER, path.to_str().unwrap(), window]);
    assert!(out.status.success(), "stamp failed: {}", stderr(&out));
    path.to_str().unwrap().to_string()
}

#[test]
fn the_helper_self_test_passes() {
    let out = python(&[HELPER, "--self-test"]);
    assert!(
        out.status.success(),
        "exit {:?}\n{}",
        out.status.code(),
        stderr(&out)
    );
}

#[test]
fn compare_refuses_mixed_recipes_and_rejects_bad_input() {
    let dir = scratch("compare");
    let first = make_stamp(&dir, "a.fp.json", "UTC day");
    let second = make_stamp(&dir, "b.fp.json", "local day");

    let same = python(&[HELPER, "compare", &first, &first]);
    assert_eq!(same.status.code(), Some(0), "{}", stderr(&same));

    let mixed = python(&[HELPER, "compare", &first, &second]);
    assert_eq!(mixed.status.code(), Some(3), "{}", stderr(&mixed));
    assert!(stderr(&mixed).contains("window"), "{}", stderr(&mixed));

    let text = fs::read_to_string(&first).unwrap();
    let edited = dir.join("edited.fp.json");
    fs::write(&edited, text.replace("UTC day", "edited")).unwrap();
    let corrupt = python(&[HELPER, "compare", edited.to_str().unwrap()]);
    assert_eq!(corrupt.status.code(), Some(1), "{}", stderr(&corrupt));

    let usage = python(&[HELPER]);
    assert_eq!(usage.status.code(), Some(2), "{}", stderr(&usage));
    let _ = fs::remove_dir_all(&dir); // best-effort temp cleanup; the asserts above are the test
}

#[test]
fn read_rate_refuses_a_changed_threshold_and_leaves_the_sidecar() {
    let dir = scratch("read-rate");
    let root = dir.join("projects");
    fs::create_dir_all(&root).unwrap();
    let entries = [
        r#"{"timestamp":"2026-11-16T10:00:00Z","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/x/big.rs"}}]}}"#,
        r#"{"timestamp":"2026-11-16T10:00:01Z","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"x"}]}}"#,
    ];
    fs::write(root.join("session.jsonl"), entries.join("\n")).unwrap();
    let sidecar = dir.join("rr.json");
    let root_arg = root.to_str().unwrap();
    let sidecar_arg = sidecar.to_str().unwrap();

    let baseline = python(&[READ_RATE, "--root", root_arg, "--fingerprint", sidecar_arg]);
    assert_eq!(baseline.status.code(), Some(0), "{}", stderr(&baseline));
    assert!(
        stderr(&baseline).contains("baseline written"),
        "{}",
        stderr(&baseline)
    );
    let before = fs::read(&sidecar).unwrap();

    let refused = python(&[
        READ_RATE,
        "--root",
        root_arg,
        "--fingerprint",
        sidecar_arg,
        "--threshold",
        "8192",
    ]);
    assert_eq!(refused.status.code(), Some(3), "{}", stderr(&refused));
    assert!(
        // The changed value itself, not only `--threshold` in the accept command.
        stderr(&refused).contains("filters.threshold: 16384 → 8192"),
        "{}",
        stderr(&refused)
    );
    assert!(refused.stdout.is_empty(), "the table was printed");
    assert_eq!(fs::read(&sidecar).unwrap(), before, "sidecar changed");

    let accepted = python(&[
        READ_RATE,
        "--root",
        root_arg,
        "--fingerprint",
        sidecar_arg,
        "--threshold",
        "8192",
        "--rebaseline",
    ]);
    assert_eq!(accepted.status.code(), Some(0), "{}", stderr(&accepted));
    let kept = fs::read_to_string(&sidecar).unwrap();
    assert!(
        kept.contains("\"rebaselines\": [\n    {") && kept.contains("\"threshold\": 16384"),
        "both recipes are kept: {kept}"
    );

    let corrupt = dir.join("corrupt.json");
    fs::write(&corrupt, "{").unwrap();
    let broken = python(&[
        READ_RATE,
        "--root",
        root_arg,
        "--fingerprint",
        corrupt.to_str().unwrap(),
    ]);
    assert_eq!(broken.status.code(), Some(1), "{}", stderr(&broken));
    assert!(
        !stderr(&broken).contains("Traceback"),
        "{}",
        stderr(&broken)
    );

    let usage = python(&[READ_RATE, "--root", root_arg, "--rebaseline"]);
    assert_eq!(usage.status.code(), Some(2), "{}", stderr(&usage));
    let _ = fs::remove_dir_all(&dir); // best-effort temp cleanup; the asserts above are the test
}
