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
    check(path, recipe, coverage, now=None) -> Baseline | Recorded
    rebaseline(path, recipe, coverage, now=None) -> Baseline | Recorded | Rebaselined
    stamp(path, recipe, coverage, now=None) -> None
    compare(paths) -> list of groups       group_stamps(paths) -> list of groups
    accept_command(argv=None) -> str

`check` raises `RecipeRefused` (nothing written) when the recipe differs from
the sidecar's; `compare` raises `StampsRefused` when the stamps hold more than
one recipe. Both are `Refused`, carry the changed keys with old and new values,
and render the message. `SidecarError` (a sidecar or stamp that is missing,
unreadable, not JSON, inconsistent, not lockable or not writable) carries the
file and the cause. The caller turns them into exit codes.

    measure_fingerprint.py compare FILE... [--rebaseline]
    measure_fingerprint.py --self-test

`compare` reads stamps written by `stamp()`: one fingerprint passes; mixed
fingerprints are refused, and with `--rebaseline` each group's fingerprint,
files and full recipe are printed on stdout. The script has no `--version`:
its version is its git blob id, which a recipe records.

Exit codes: 0 done (also when stdout is closed early, as by `| head`); 1 a
stamp or sidecar is missing, unreadable, not JSON or inconsistent (its stored
fingerprint does not match its recipe, even with `--rebaseline`); 2 usage
error; 3 fingerprints differ (refused).

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
from typing import NamedTuple

__all__ = [
    "Baseline", "Recorded", "Rebaselined", "Refused", "RecipeRefused",
    "StampsRefused", "SidecarError", "accept_command", "blob_id", "canonical",
    "check", "compare", "fingerprint", "group_stamps", "rebaseline", "recipe",
    "stamp",
]

LOCK_WAIT_SECONDS = 30.0
LOCK_POLL_SECONDS = 0.05
WIDTH = 80
RECIPE_KEYS = ("script", "tool_version", "source", "schema", "filters", "window")


class Baseline(NamedTuple):
    """No sidecar existed: this run is its baseline, run 1."""
    path: str


class Recorded(NamedTuple):
    """Same recipe: run number `run` was appended."""
    path: str
    run: int


class Rebaselined(NamedTuple):
    """A changed recipe was accepted: both kept, run number `run` appended."""
    path: str
    run: int
    keys: list


class Refused(Exception):
    """A recipe differs from the one it is compared with. `changes` is a list
    of (key, old, new); `render(accept)` is the message for the person."""

    def __init__(self, changes: list):
        super().__init__("fingerprint refused")
        self.changes = changes

    def _header(self) -> list:
        raise NotImplementedError

    def _footer(self) -> list:
        return []

    def render(self, accept: str | None = None) -> str:
        lines = self._header()
        for key, old, new in self.changes:
            lines.extend(_change_lines(key, old, new))
        lines.extend(self._footer())
        if accept is not None:
            lines.append("to accept the new recipe, run:")
            lines.append(f"  {accept}")
        return "\n".join(lines)

    def __str__(self) -> str:
        return self.render()


class RecipeRefused(Refused):
    """`check`: the sidecar at `path` records another recipe."""

    def __init__(self, changes: list, path: str, old_fp: str, new_fp: str):
        super().__init__(changes)
        self.path = path
        self.old_fp = old_fp
        self.new_fp = new_fp

    def _header(self) -> list:
        return [
            f"fingerprint refused: recipe changed "
            f"({_short(self.old_fp)} → {_short(self.new_fp)})"
        ]

    def _footer(self) -> list:
        # Never cut: a path is copied, like the accept command.
        return [f"sidecar: {self.path}"]


class StampsRefused(Refused):
    """`compare`: the stamps hold more than one recipe; `groups` is a list of
    {"fingerprint", "files", "recipe"}."""

    def __init__(self, changes: list, groups: list):
        super().__init__(changes)
        self.groups = groups

    def _header(self) -> list:
        lines = [
            f"fingerprints differ: {len(self.groups)} recipes in "
            f"{sum(len(g['files']) for g in self.groups)} files"
        ]
        for group in self.groups:
            lines.append(f"  {_short(group['fingerprint'])}")
            for name in group["files"]:
                # Never cut: the name tells the pieces apart (a day).
                lines.append(f"    {name}")
        return lines


class SidecarError(Exception):
    def __init__(self, path, cause):
        super().__init__(f"{path}: {cause}")
        self.path = str(path)
        self.cause = str(cause)


def accept_command(argv: list | None = None) -> str:
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
    raise TypeError(f"recipe or coverage value is not JSON: {type(value).__name__}")


def recipe(script, tool_version: str, source: str, schema: str, filters: dict,
           window: str) -> dict:
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


def canonical(recipe_dict: dict) -> str:
    return json.dumps(
        _plain(recipe_dict), sort_keys=True, separators=(",", ":"), ensure_ascii=False
    )


def fingerprint(recipe_dict: dict) -> str:
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
    except FileNotFoundError as err:
        raise SidecarError(path, "no such file") from err
    except (OSError, UnicodeDecodeError) as err:
        raise SidecarError(path, f"cannot be read ({err})") from err
    except ValueError as err:
        raise SidecarError(path, f"is not JSON ({err})") from err


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
        raise SidecarError(path, f"recipe is not canonical ({err})") from err
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
    """Temp file in the same directory, fsync, rename, fsync the directory;
    an existing file keeps its mode (mkstemp makes 0600)."""
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
        try:
            os.chmod(temp, os.stat(path).st_mode & 0o7777)
        except FileNotFoundError:
            os.chmod(temp, 0o644)
        os.replace(temp, path)
        temp = None
        folder = os.open(directory, os.O_RDONLY)
        try:
            os.fsync(folder)
        finally:
            os.close(folder)
    except OSError as err:
        raise SidecarError(path, f"cannot be written ({err})") from err
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
            self.handle = open(self.lock_path, "ab")
        except OSError as err:
            raise SidecarError(self.lock_path, f"cannot be opened ({err})") from err
        deadline = time.monotonic() + LOCK_WAIT_SECONDS
        while True:
            try:
                fcntl.flock(self.handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                return self
            except BlockingIOError as err:  # held by another run: wait, bounded
                if time.monotonic() >= deadline:
                    self.handle.close()
                    raise SidecarError(
                        self.lock_path, f"still locked after {LOCK_WAIT_SECONDS:.0f} s"
                    ) from err
                time.sleep(LOCK_POLL_SECONDS)
            except OSError as err:  # not contention: say what it is, at once
                self.handle.close()
                raise SidecarError(self.lock_path, f"cannot be locked ({err})") from err

    def __exit__(self, *exc):
        self.handle.close()
        return False


def _record(path, recipe_dict, coverage, now, accept_change):
    path = str(path)
    recipe_dict = _plain(recipe_dict)
    coverage = _plain(coverage)
    new_fp = fingerprint(recipe_dict)
    stamp_time = _now(now)
    run = {"at": stamp_time, "coverage": coverage}
    with _Locked(path):
        if not os.path.exists(path):
            doc = {"fingerprint": new_fp, "recipe": recipe_dict, "runs": [run],
                   "rebaselines": []}
            _write_atomic(path, doc)
            return Baseline(path)
        doc = _load_sidecar(path)
        if doc["fingerprint"] == new_fp:
            doc["runs"].append(run)
            _write_atomic(path, doc)
            return Recorded(path, len(doc["runs"]))
        changes = _changes(doc["recipe"], recipe_dict)
        if not accept_change:
            raise RecipeRefused(changes, path, doc["fingerprint"], new_fp)
        doc["rebaselines"].append(
            {
                "at": stamp_time,
                "from": {"fingerprint": doc["fingerprint"], "recipe": doc["recipe"]},
                "to": {"fingerprint": new_fp, "recipe": recipe_dict},
            }
        )
        doc["fingerprint"] = new_fp
        doc["recipe"] = recipe_dict
        doc["runs"].append(run)
        _write_atomic(path, doc)
        return Rebaselined(path, len(doc["runs"]), [key for key, _, _ in changes])


def check(path, recipe_dict: dict, coverage: dict, now: str | None = None):
    """Record a run against `path`'s baseline: Baseline or Recorded. A changed
    recipe raises RecipeRefused and writes nothing."""
    return _record(path, recipe_dict, coverage, now, accept_change=False)


def rebaseline(path, recipe_dict: dict, coverage: dict, now: str | None = None):
    """As `check`, but a changed recipe is accepted: both recipes are kept in
    `rebaselines` and Rebaselined is returned."""
    return _record(path, recipe_dict, coverage, now, accept_change=True)


def stamp(path, recipe_dict: dict, coverage: dict, now: str | None = None) -> None:
    """Write one run's sidecar, for a measure captured in pieces."""
    path = str(path)
    recipe_dict = _plain(recipe_dict)
    doc = {
        "fingerprint": fingerprint(recipe_dict),
        "recipe": recipe_dict,
        "coverage": _plain(coverage),
        "at": _now(now),
    }
    with _Locked(path):
        _write_atomic(path, doc)


def group_stamps(paths: list) -> list:
    """The stamps grouped by fingerprint, first seen first; never refuses."""
    groups = {}
    for path in paths:
        doc = _load_stamp(str(path))
        group = groups.setdefault(
            doc["fingerprint"],
            {"fingerprint": doc["fingerprint"], "files": [], "recipe": doc["recipe"]},
        )
        group["files"].append(str(path))
    return list(groups.values())


def compare(paths: list) -> list:
    """The stamps' single group; StampsRefused when they hold more than one."""
    ordered = group_stamps(paths)
    if len(ordered) > 1:
        changes = []
        for other in ordered[1:]:
            for change in _changes(ordered[0]["recipe"], other["recipe"]):
                if change not in changes:
                    changes.append(change)
        raise StampsRefused(changes, ordered)
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
        if args.rebaseline:
            _print_groups(group_stamps(args.files))
        else:
            compare(args.files)
    except Refused as refused:
        print(refused.render(accept_command()), file=sys.stderr)
        return 3
    except SidecarError as err:
        print(f"measure_fingerprint: {err}", file=sys.stderr)
        return 1
    return 0


def _main(argv) -> int:
    try:
        code = _cli(argv)
        sys.stdout.flush()
        return code
    except BrokenPipeError:
        # The reader went away (`| head`). Point stdout at the null device so
        # the interpreter's exit flush does not raise again, and leave quietly.
        try:
            os.dup2(os.open(os.devnull, os.O_WRONLY), sys.stdout.fileno())
        except OSError:
            pass  # stdout is already unusable; exit 0 is still right
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
    _expect(isinstance(first, Baseline), "baseline kind")
    doc = json.loads(read_bytes("s.json"))
    _expect(len(doc["runs"]) == 1 and doc["runs"][0]["at"] == "t1", "baseline is run 1")
    _expect(doc["rebaselines"] == [], "no rebaselines yet")
    _expect(check(at("s.json"), base, coverage, now="t2") == Recorded(at("s.json"), 2),
            "same fp run 2")

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

    _raises(RecipeRefused, "refusal is a RecipeRefused", check, at("s.json"), changed, coverage)
    rebased = rebaseline(at("s.json"), changed, coverage, now="t4")
    _expect(rebased == Rebaselined(at("s.json"), 3, ["filters.ext"]), f"rebaseline: {rebased}")
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
        SidecarError, "edited sidecar", rebaseline, at("s-edited.json"), changed, coverage,
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

    _raises(TypeError, "non-JSON coverage", stamp, at("c.fp.json"), base, {"x": object()})
    _expect(json.loads(read_bytes("a.fp.json"))["coverage"] == coverage, "coverage kept")
    mode = os.stat(at("s.json")).st_mode & 0o777
    os.chmod(at("s.json"), 0o640)
    check(at("s.json"), changed, coverage)
    _expect(os.stat(at("s.json")).st_mode & 0o777 == 0o640, f"mode kept (was {mode:o})")
    _raises(StampsRefused, "mixed stamps", compare, [at("a.fp.json"), at("b.fp.json")])
    _expect(len(group_stamps([at("a.fp.json"), at("b.fp.json")])) == 2, "group_stamps")
    piped = subprocess.run(
        f"{shlex.quote(sys.executable)} -I {shlex.quote(os.path.abspath(__file__))} "
        # 3,000 names print well past a pipe buffer, so `head` closing early
        # really raises EPIPE in the writer.
        f"compare {' '.join([shlex.quote(at('a.fp.json'))] * 3000)} "
        f"{shlex.quote(at('b.fp.json'))} --rebaseline | head -c 1 >/dev/null",
        shell=True, capture_output=True, text=True, timeout=60,
    )
    _expect("Traceback" not in piped.stderr, f"closed stdout: {piped.stderr[-200:]}")


if __name__ == "__main__":
    sys.exit(_main(sys.argv[1:]))
