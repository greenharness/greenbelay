#!/usr/bin/env python3
"""A fingerprint for a dated measure: refuse to compare counts made by
different recipes.

A measure that is re-run over days or weeks and compared to an earlier number
records its recipe (the script blob, the tool version, the source, the schema
marker, the filters, the window rule) in a sidecar JSON file. The recipe is
hashed, never the data. A later run with a different recipe is refused unless
the caller accepts it with `--rebaseline`, which keeps both recipes on record.

Library, loaded by a caller by path (`importlib.util.spec_from_file_location`
beside the caller, never `import`: `python3 -I` drops the script's directory
from `sys.path`):

    recipe(script, tool_version, source, schema, filters, window) -> dict
    canonical(recipe) -> str        fingerprint(recipe) -> "sha256:<hex>"
    check(path, recipe, coverage, rebaseline=False, now=None) -> dict
    stamp(path, recipe, coverage, now=None) -> None
    compare(paths, rebaseline=False) -> list of groups

`Refused` (a different recipe) carries the changed keys with old and new
values and the sidecar path; `SidecarError` (a sidecar that is missing,
unreadable, not JSON, inconsistent or not writable) carries the file and the
cause. Both are for the caller to turn into an exit code.

    measure_fingerprint.py compare FILE... [--rebaseline]
    measure_fingerprint.py --self-test

`compare` reads stamps written by `stamp()`: one fingerprint passes; mixed
fingerprints are refused, and with `--rebaseline` each group's fingerprint,
files and full recipe are printed on stdout.

Exit codes: 0 done; 1 a stamp or sidecar is missing, unreadable, not JSON or
inconsistent (its stored fingerprint does not match its recipe, even with
`--rebaseline`); 2 usage error; 3 fingerprints differ (refused).

Sidecar writes take `fcntl.flock` on `<sidecar>.lock`; `fcntl` is Unix-only,
so this helper is too.
"""

import argparse
import datetime
import fcntl
import hashlib
import json
import os
import shlex
import subprocess
import sys
import tempfile
import time

LOCK_WAIT_SECONDS = 30.0
LOCK_POLL_SECONDS = 0.05
WIDTH = 80
RECIPE_KEYS = ("script", "tool_version", "source", "schema", "filters", "window")


class Refused(Exception):
    """The recipe differs from the recorded one.

    `changes` is a list of (key, old, new); `path` is the sidecar (or the
    stamps' directory-less description for `compare`); `groups` is set by
    `compare`: a list of {"fingerprint", "files", "recipe"}.
    """

    def __init__(self, changes, path=None, groups=None, old_fp=None, new_fp=None):
        super().__init__("fingerprint refused")
        self.changes = changes
        self.path = path
        self.groups = groups
        self.old_fp = old_fp
        self.new_fp = new_fp

    def render(self, accept=None) -> str:
        lines = []
        if self.groups is not None:
            lines.append(
                f"fingerprints differ: {len(self.groups)} recipes in "
                f"{sum(len(g['files']) for g in self.groups)} files"
            )
            for group in self.groups:
                lines.append(f"  {_short(group['fingerprint'])}")
                for name in group["files"]:
                    # Never cut: the name tells the pieces apart (a day).
                    lines.append(f"    {name}")
        else:
            lines.append(
                f"fingerprint refused: recipe changed "
                f"({_short(self.old_fp)} → {_short(self.new_fp)})"
            )
        for key, old, new in self.changes:
            lines.extend(_change_lines(key, old, new))
        if self.path is not None:
            # Never cut: a path is copied, like the accept command below.
            lines.append(f"sidecar: {self.path}")
        if accept is not None:
            lines.append("to accept the new recipe, run:")
            lines.append(f"  {accept}")
        return "\n".join(lines)

    def __str__(self) -> str:
        return self.render()


class SidecarError(Exception):
    def __init__(self, path, cause):
        super().__init__(f"{path}: {cause}")
        self.path = str(path)
        self.cause = str(cause)


def accept_command(argv=None) -> str:
    """The caller's own argv plus `--rebaseline`, to accept a refusal.

    By default the running command: the interpreter, its flags (`-I` kept,
    from `sys.orig_argv`) and the script's arguments.
    """
    if argv is None:
        argv = [sys.executable, *sys.orig_argv[1:]]
    return shlex.join([*argv, "--rebaseline"])


def _short(fp) -> str:
    text = str(fp)
    if text.startswith("sha256:"):
        text = text[len("sha256:"):]
    return text[:12]


def _fit(line: str) -> str:
    if len(line) <= WIDTH:
        return line
    return line[: WIDTH - 1] + "…"


def _show(value) -> str:
    if isinstance(value, str):
        return value
    return json.dumps(value, sort_keys=True, ensure_ascii=False)


def _change_lines(key, old, new):
    if key == "tool_version":
        return [_fit(f"  {key}: interpreter changed ({_show(old)} → {_show(new)})")]
    one = f"  {key}: {_show(old)} → {_show(new)}"
    if len(one) <= WIDTH:
        return [one]
    return [
        f"  {key}:",
        _fit(f"    was: {_show(old)}"),
        _fit(f"    now: {_show(new)}"),
    ]


def blob_id(data: bytes) -> str:
    """The git blob id: sha1 of `blob <n>\\0<bytes>`."""
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def _plain(value):
    if isinstance(value, (set, frozenset)):
        items = [_plain(item) for item in value]
        return sorted(items, key=lambda item: json.dumps(item, sort_keys=True))
    if isinstance(value, dict):
        return {str(key): _plain(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_plain(item) for item in value]
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    raise TypeError(f"recipe value is not JSON: {type(value).__name__}")


def recipe(script, tool_version, source, schema, filters, window) -> dict:
    with open(script, "rb") as handle:
        blob = blob_id(handle.read())
    return _plain(
        {
            "script": blob,
            "tool_version": tool_version,
            "source": source,
            "schema": schema,
            "filters": filters,
            "window": window,
        }
    )


def canonical(recipe_dict) -> str:
    return json.dumps(
        _plain(recipe_dict), sort_keys=True, separators=(",", ":"), ensure_ascii=False
    )


def fingerprint(recipe_dict) -> str:
    return "sha256:" + hashlib.sha256(canonical(recipe_dict).encode("utf-8")).hexdigest()


def _now(now):
    if now is not None:
        return now
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")


def _changes(old_recipe, new_recipe, prefix=""):
    """(key, old, new) per changed key; a dict on both sides is walked, so a
    change inside `filters` is named `filters.threshold`, not the whole dict."""
    keys = [key for key in RECIPE_KEYS if key in old_recipe or key in new_recipe]
    keys += sorted((set(old_recipe) | set(new_recipe)) - set(keys))
    found = []
    for key in keys:
        old, new = old_recipe.get(key), new_recipe.get(key)
        if old == new:
            continue
        if isinstance(old, dict) and isinstance(new, dict):
            found.extend(_changes(old, new, f"{prefix}{key}."))
        else:
            found.append((f"{prefix}{key}", old, new))
    return found


def _read_json(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except FileNotFoundError:
        raise SidecarError(path, "no such file") from None
    except (OSError, UnicodeDecodeError) as err:
        raise SidecarError(path, f"cannot be read ({err})") from None
    except ValueError as err:
        raise SidecarError(path, f"is not JSON ({err})") from None


def _verified(path, doc, required):
    if not isinstance(doc, dict):
        raise SidecarError(path, "is not a JSON object")
    for key in required:
        if key not in doc:
            raise SidecarError(path, f"is missing the key `{key}`")
    if not isinstance(doc["recipe"], dict):
        raise SidecarError(path, "`recipe` is not an object")
    try:
        expected = fingerprint(doc["recipe"])
    except TypeError as err:
        raise SidecarError(path, f"recipe is not canonical ({err})") from None
    if doc["fingerprint"] != expected:
        raise SidecarError(path, "stored fingerprint does not match its recipe")
    return doc


def _load_sidecar(path):
    doc = _verified(
        path, _read_json(path), ("fingerprint", "recipe", "runs", "rebaselines")
    )
    if not isinstance(doc["runs"], list) or not isinstance(doc["rebaselines"], list):
        raise SidecarError(path, "`runs` and `rebaselines` must be lists")
    return doc


def _load_stamp(path):
    return _verified(path, _read_json(path), ("fingerprint", "recipe", "coverage", "at"))


def _write_atomic(path, doc):
    directory = os.path.dirname(os.path.abspath(path))
    name = os.path.basename(path)
    temp = None
    try:
        descriptor, temp = tempfile.mkstemp(dir=directory, prefix=name + ".", suffix=".tmp")
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(doc, handle, indent=2, sort_keys=True, ensure_ascii=False)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp, path)
        temp = None
    except OSError as err:
        raise SidecarError(path, f"cannot be written ({err})") from None
    finally:
        if temp is not None:
            try:
                os.unlink(temp)
            except OSError:
                pass  # the temp file is already gone; the write error is what matters


class _Locked:
    def __init__(self, path):
        self.lock_path = f"{path}.lock"
        self.handle = None

    def __enter__(self):
        try:
            self.handle = open(self.lock_path, "a")
        except OSError as err:
            raise SidecarError(self.lock_path, f"cannot be opened ({err})") from None
        deadline = time.monotonic() + LOCK_WAIT_SECONDS
        while True:
            try:
                fcntl.flock(self.handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                return self
            except OSError:
                if time.monotonic() >= deadline:
                    self.handle.close()
                    raise SidecarError(
                        self.lock_path, f"still locked after {LOCK_WAIT_SECONDS:.0f} s"
                    ) from None
                time.sleep(LOCK_POLL_SECONDS)

    def __exit__(self, *exc):
        self.handle.close()
        return False


def check(path, recipe_dict, coverage, rebaseline=False, now=None) -> dict:
    """Record a run against `path`'s baseline.

    Returns {"kind": "baseline" | "run" | "rebaseline", "run": N, "keys": [...]}.
    Raises Refused (nothing written) when the recipe differs and not `rebaseline`.
    """
    path = str(path)
    recipe_dict = _plain(recipe_dict)
    new_fp = fingerprint(recipe_dict)
    stamp_time = _now(now)
    with _Locked(path):
        if not os.path.exists(path):
            doc = {
                "fingerprint": new_fp,
                "recipe": recipe_dict,
                "runs": [{"at": stamp_time, "coverage": coverage}],
                "rebaselines": [],
            }
            _write_atomic(path, doc)
            return {"kind": "baseline", "run": 1, "keys": []}
        doc = _load_sidecar(path)
        if doc["fingerprint"] == new_fp:
            doc["runs"].append({"at": stamp_time, "coverage": coverage})
            _write_atomic(path, doc)
            return {"kind": "run", "run": len(doc["runs"]), "keys": []}
        changes = _changes(doc["recipe"], recipe_dict)
        if not rebaseline:
            raise Refused(
                changes, path=path, old_fp=doc["fingerprint"], new_fp=new_fp
            )
        doc["rebaselines"].append(
            {
                "at": stamp_time,
                "from": {"fingerprint": doc["fingerprint"], "recipe": doc["recipe"]},
                "to": {"fingerprint": new_fp, "recipe": recipe_dict},
            }
        )
        doc["fingerprint"] = new_fp
        doc["recipe"] = recipe_dict
        doc["runs"].append({"at": stamp_time, "coverage": coverage})
        _write_atomic(path, doc)
        return {
            "kind": "rebaseline",
            "run": len(doc["runs"]),
            "keys": [key for key, _, _ in changes],
        }


def stamp(path, recipe_dict, coverage, now=None) -> None:
    """Write one run's sidecar, for a measure captured in pieces."""
    path = str(path)
    recipe_dict = _plain(recipe_dict)
    doc = {
        "fingerprint": fingerprint(recipe_dict),
        "recipe": recipe_dict,
        "coverage": coverage,
        "at": _now(now),
    }
    with _Locked(path):
        _write_atomic(path, doc)


def compare(paths, rebaseline=False) -> list:
    """Group stamps by fingerprint; raise Refused when mixed unless `rebaseline`."""
    groups = {}
    for path in paths:
        doc = _load_stamp(str(path))
        group = groups.setdefault(
            doc["fingerprint"],
            {"fingerprint": doc["fingerprint"], "files": [], "recipe": doc["recipe"]},
        )
        group["files"].append(str(path))
    ordered = list(groups.values())
    if len(ordered) > 1 and not rebaseline:
        changes = []
        for other in ordered[1:]:
            for change in _changes(ordered[0]["recipe"], other["recipe"]):
                if change not in changes:
                    changes.append(change)
        raise Refused(changes, groups=ordered)
    return ordered


def _print_groups(groups):
    for group in groups:
        print(group["fingerprint"])
        for name in group["files"]:
            print(f"  {name}")
        print(f"  recipe: {canonical(group['recipe'])}")


def _cli(argv) -> int:
    ap = argparse.ArgumentParser(
        prog="measure_fingerprint.py",
        description="Compare the fingerprints of dated measures' stamps.",
    )
    ap.add_argument("command", nargs="?", choices=["compare"])
    ap.add_argument("files", nargs="*")
    ap.add_argument("--rebaseline", action="store_true",
                    help="accept mixed fingerprints and print each recipe")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_intermixed_args(argv)
    if args.self_test:
        return _self_test()
    if args.command != "compare" or not args.files:
        ap.error("usage: compare FILE... [--rebaseline]")
    try:
        groups = compare(args.files, args.rebaseline)
    except Refused as refused:
        print(
            refused.render(accept_command()),
            file=sys.stderr,
        )
        return 3
    except SidecarError as err:
        print(f"measure_fingerprint: {err}", file=sys.stderr)
        return 1
    if args.rebaseline:
        _print_groups(groups)
    return 0


class _SelfTestFailure(Exception):
    pass


def _expect(condition, label):
    if not condition:
        raise _SelfTestFailure(label)


def _raises(kind, label, function, *args, **kwargs):
    try:
        function(*args, **kwargs)
    except kind as err:
        return err
    raise _SelfTestFailure(f"{label}: {kind.__name__} not raised")


def _run_cli(*args):
    return subprocess.run(
        [sys.executable, "-I", os.path.abspath(__file__), *args],
        capture_output=True,
        text=True,
        timeout=60,
    )


def _self_test() -> int:
    try:
        with tempfile.TemporaryDirectory() as directory:
            _self_cases(directory)
    except _SelfTestFailure as err:
        print(f"measure_fingerprint: self-test failed: {err}", file=sys.stderr)
        return 1
    print("measure_fingerprint: self-test ok", file=sys.stderr)
    return 0


def _self_cases(directory):
    def at(name):
        return os.path.join(directory, name)

    def read_bytes(name):
        with open(at(name), "rb") as handle:
            return handle.read()

    def write_text(name, text):
        with open(at(name), "w", encoding="utf-8") as handle:
            handle.write(text)

    _expect(blob_id(b"") == "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391", "empty blob")
    _expect(
        blob_id(b"hello\n") == "ce013625030ba8dba906f756967f9e9ca394464a", "hello blob"
    )

    write_text("script.py", "hello\n")
    base = recipe(
        at("script.py"), "python3.14", "src", "schema",
        {"ext": {"png", "gif"}, "limit": 3}, "UTC day",
    )
    _expect(base["script"] == "ce013625030ba8dba906f756967f9e9ca394464a", "recipe blob")
    _expect(base["filters"]["ext"] == ["gif", "png"], "sets are sorted lists")
    _expect('"ext":["gif","png"]' in canonical(base), "canonical sorted set")
    _expect(fingerprint(base).startswith("sha256:"), "fingerprint prefix")
    _raises(TypeError, "non-JSON value", canonical, {"x": object()})

    changed = dict(base, filters={"ext": ["gif"], "limit": 3})
    far = dict(base, window="x" * 120)
    coverage = {"files": 2}

    first = check(at("s.json"), base, coverage, now="t1")
    _expect(first["kind"] == "baseline", "baseline kind")
    doc = json.loads(read_bytes("s.json"))
    _expect(len(doc["runs"]) == 1 and doc["runs"][0]["at"] == "t1", "baseline is run 1")
    _expect(doc["rebaselines"] == [], "no rebaselines yet")
    _expect(check(at("s.json"), base, coverage, now="t2")["run"] == 2, "same fp run 2")

    before = read_bytes("s.json")
    refusal = _raises(Refused, "refusal", check, at("s.json"), changed, coverage, now="t3")
    _expect(read_bytes("s.json") == before, "refusal writes nothing")
    _expect(
        refusal.changes == [("filters.ext", ["gif", "png"], ["gif"])],
        f"changed keys name the nested key: {refusal.changes}",
    )
    _expect("  filters.ext: " in refusal.render(), "nested change on one line")
    _expect(refusal.path == at("s.json"), "refusal carries the path")
    text = refusal.render("cmd --rebaseline")
    _expect("filters.ext:" in text and "cmd --rebaseline" in text, "refusal text")
    _expect(f"sidecar: {at('s.json')}" in text, "the sidecar path is whole")
    long_refusal = _raises(Refused, "long refusal", check, at("s.json"), far, coverage)
    _expect(
        all(len(line) <= WIDTH for line in long_refusal.render().splitlines()),
        "refusal lines fit 80 columns",
    )
    _expect("was:" in long_refusal.render() and "now:" in long_refusal.render(), "was/now lines")
    interp = _raises(
        Refused, "interpreter", check, at("s.json"),
        dict(base, tool_version="python3.15"), coverage,
    )
    _expect("interpreter changed" in interp.render(), "interpreter changed")

    rebased = check(at("s.json"), changed, coverage, rebaseline=True, now="t4")
    _expect(rebased["kind"] == "rebaseline" and rebased["keys"] == ["filters.ext"], "rebaseline")
    doc = json.loads(read_bytes("s.json"))
    _expect(doc["fingerprint"] == fingerprint(changed), "new baseline")
    _expect(len(doc["rebaselines"]) == 1 and len(doc["runs"]) == 3, "rebaseline appended")
    entry = doc["rebaselines"][0]
    _expect(
        entry["from"]["recipe"] == base and entry["to"]["recipe"] == changed,
        "both recipes kept",
    )

    names = os.listdir(directory)
    _expect(not any(n.endswith(".tmp") for n in names), "no mkstemp temp left")
    _expect("s.json.lock" in names, "lock file stays")

    stamp(at("a.fp.json"), base, coverage, now="t1")
    stamp(at("a2.fp.json"), base, coverage, now="t1")
    _expect(read_bytes("a.fp.json") == read_bytes("a2.fp.json"), "now= is deterministic")
    stamp(at("b.fp.json"), changed, coverage, now=None)
    stamp(at("null.fp.json"), base, coverage, now=None)
    doc = json.loads(read_bytes("null.fp.json"))
    doc["at"] = None
    write_text("null.fp.json", json.dumps(doc))
    _expect(len(compare([at("a.fp.json"), at("null.fp.json")])) == 1, "at: null accepted")

    ran = _run_cli("compare", at("a.fp.json"), at("a2.fp.json"))
    _expect(ran.returncode == 0, f"compare one group: {ran.returncode}")
    mixed = _run_cli("compare", at("a.fp.json"), at("b.fp.json"))
    _expect(mixed.returncode == 3 and "filters" in mixed.stderr, "compare mixed exit 3")
    _expect(mixed.stdout == "", "refusal prints nothing on stdout")
    files = {at("a.fp.json"), at("b.fp.json")}
    _expect(
        all(
            len(line) <= WIDTH
            for line in mixed.stderr.splitlines()[:-1]
            if line.strip() not in files
        ),
        "compare refusal fits 80 columns, file names aside",
    )
    _expect(
        all(f"    {name}" in mixed.stderr.splitlines() for name in files),
        "file names are whole",
    )
    _expect("--rebaseline" in mixed.stderr, "compare refusal names the accept command")
    accepted = _run_cli("compare", at("a.fp.json"), at("b.fp.json"), "--rebaseline")
    _expect(accepted.returncode == 0, "compare --rebaseline exit 0")
    _expect(
        fingerprint(base) in accepted.stdout
        and fingerprint(changed) in accepted.stdout
        and at("b.fp.json") in accepted.stdout
        and canonical(changed) in accepted.stdout,
        "rebaseline prints each group",
    )
    _expect(_run_cli("compare").returncode == 2, "compare with no files exits 2")
    _expect(_run_cli().returncode == 2, "no command exits 2")

    doc = json.loads(read_bytes("a.fp.json"))
    doc["recipe"]["window"] = "edited by hand"
    write_text("edited.fp.json", json.dumps(doc))
    _raises(SidecarError, "edited stamp", compare, [at("edited.fp.json")])
    forced = _run_cli("compare", at("edited.fp.json"), "--rebaseline")
    _expect(forced.returncode == 1, "edited recipe exits 1 even with --rebaseline")
    _expect("does not match its recipe" in forced.stderr, "corruption message")
    doc = json.loads(read_bytes("s.json"))
    doc["recipe"]["window"] = "edited by hand"
    write_text("s-edited.json", json.dumps(doc))
    _raises(
        SidecarError, "edited sidecar", check, at("s-edited.json"), changed, coverage,
        rebaseline=True,
    )

    write_text("bad.json", "{")
    bad = _run_cli("compare", at("bad.json"))
    _expect(bad.returncode == 1, "corrupt stamp exits 1")
    _expect(
        "Traceback" not in bad.stderr and len(bad.stderr.strip().splitlines()) == 1,
        "corrupt stamp: one line, no traceback",
    )
    _expect("bad.json" in bad.stderr, "corrupt stamp names the file")
    _raises(SidecarError, "corrupt sidecar", check, at("bad.json"), base, coverage)
    write_text("partial.json", json.dumps({"fingerprint": "x"}))
    _raises(SidecarError, "missing keys", check, at("partial.json"), base, coverage)
    missing = _run_cli("compare", at("absent.json"))
    _expect(missing.returncode == 1 and "Traceback" not in missing.stderr, "missing stamp")
    names = os.listdir(directory)
    _expect(not any(n.endswith(".tmp") for n in names), "no temp left after failures")


if __name__ == "__main__":
    sys.exit(_cli(sys.argv[1:]))
