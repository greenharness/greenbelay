#!/bin/bash
# Which tests would touch the machine they run on?
#
# Every greenbelay subcommand except `migrate` and `doctor` migrates the
# machine first, so a test that starts the built binary without pointing it at
# a scratch config directory, state home and git config migrates the REAL home
# of whoever runs `cargo test`; a unit test that writes the journal or settings
# without a scratch directory writes into it. This script runs each test target
# (or the ones named) against a canary old-style machine whose every path is a
# scratch directory, and fails when a target changed the canary.
#
#   scripts/leak-hunt.sh                       every target
#   scripts/leak-hunt.sh block shipped         those tests/*.rs files
#   scripts/leak-hunt.sh lib:greenbelay-guard  a crate's unit tests
#
# Targets: each tests/<name>.rs by its name, lib:greenbelay-guard,
# lib:greenbelay-process and bin:greenbelay (the binary's own unit tests).
#
# A canary changed means: the listing of its claude/, state/ and home/ trees
# (each path, and a checksum of each file) differs after the run, which covers
# a done-marker, an old root turned into a symlink, a new state root, and any
# file a test wrote there; or the old key `amont.agent.enabled` is gone from its
# git config. A target whose `cargo test` fails to build or run (no
# `test result:` line) is BUILD-FAIL; one whose tests ran and failed is
# TEST-FAIL, and its canary is still checked, so a failing test that leaks is
# also LEAK; a canary that cannot be listed is SNAPSHOT-FAIL. None is ever ok. See also tests/spawn_isolation.rs, the
# static check. A test that only writes its own `greenbelay.*` keys into the
# inherited git config is not a leak and is not reported.
#
# LEAK_HUNT_SNAPSHOT_FAIL=1 makes every listing fail, to check that a failed
# listing is reported and never read as ok.
set -u
set -o pipefail
root=$(git rev-parse --show-toplevel) || exit 2
cd "$root" || exit 2
real_home=$HOME

usage() {
    echo "usage: scripts/leak-hunt.sh [<tests/*.rs name> | lib:greenbelay-guard | lib:greenbelay-process | bin:greenbelay]..." >&2
    exit 2
}

# The cargo arguments of a target, one per line; `--features test-hooks` only
# for the packages that declare it (the root package and greenbelay-guard).
# Unit tests run on one thread: some set CLAUDE_CONFIG_DIR process-wide while
# they run, and a test writing beside them in parallel would land in their
# scratch dir, hiding its leak from the canary.
cargo_args() {
    case "$1" in
        lib:greenbelay-guard) printf '%s\n' -p greenbelay-guard --lib --features test-hooks --no-fail-fast -- --test-threads=1 ;;
        lib:greenbelay-process) printf '%s\n' -p greenbelay-process --lib --no-fail-fast -- --test-threads=1 ;;
        bin:greenbelay) printf '%s\n' -p greenbelay --bins --features test-hooks --no-fail-fast -- --test-threads=1 ;;
        *) printf '%s\n' --features test-hooks --test "$1" --no-fail-fast ;;
    esac
}

if [ "$#" -gt 0 ]; then
    targets=("$@")
else
    targets=()
    while IFS= read -r name; do targets+=("$name"); done \
        < <(cd tests && command ls ./*.rs | sed 's|^\./||;s|\.rs$||' | sort)
    targets+=(lib:greenbelay-guard lib:greenbelay-process bin:greenbelay)
fi
# Every name is checked before the first target runs, so a usage error runs nothing.
for t in "${targets[@]}"; do
    case "$t" in
        lib:greenbelay-guard | lib:greenbelay-process | bin:greenbelay) ;;
        lib:* | bin:*) echo "leak-hunt: unknown target $t" >&2; usage ;;
        *) [ -f "tests/$t.rs" ] || { echo "leak-hunt: no tests/$t.rs" >&2; usage; } ;;
    esac
done

# The sorted listing of a canary's trees: each path, a checksum of each file.
# Only POSIX find and cksum, so it reads the same on macOS and Linux.
snapshot() {
    [ "${LEAK_HUNT_SNAPSHOT_FAIL:-}" = 1 ] && return 1
    (
        cd "$1" || exit 1
        find claude state home -print | LC_ALL=C sort | while IFS= read -r path; do
            if [ -L "$path" ]; then
                printf '%s link\n' "$path"
            elif [ -f "$path" ]; then
                sum=$(cksum < "$path") || exit 1
                printf '%s %s\n' "$path" "$sum"
            else
                printf '%s dir\n' "$path"
            fi
        done
    )
}

failures=0
canary=
# An interrupted run removes the canary it was filling; nothing else is touched.
trap '[ -n "$canary" ] && rm -rf "$canary"; exit 130' INT TERM
for t in "${targets[@]}"; do
    canary=$(mktemp -d "${TMPDIR:-/tmp}/greenbelay-canary.XXXXXX")
    mkdir -p "$canary/home" "$canary/claude/amont-agent" "$canary/state/amont-agent"
    printf 'amont-agent-v1\n' > "$canary/claude/amont-agent/journal"
    printf '[amont "agent"]\n\tenabled = true\n' > "$canary/gitconfig"
    if ! snapshot "$canary" > "$canary/before.txt"; then
        echo "SNAPSHOT-FAIL  $t (the canary could not be listed before the run; kept at $canary)"
        failures=$((failures + 1))
        canary=
        continue
    fi
    args=()
    while IFS= read -r arg; do args+=("$arg"); done < <(cargo_args "$t")
    env HOME="$canary/home" CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}" \
        RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}" \
        CLAUDE_CONFIG_DIR="$canary/claude" XDG_STATE_HOME="$canary/state" \
        XDG_CONFIG_HOME="$canary/xdg" GIT_CONFIG_GLOBAL="$canary/gitconfig" \
        GIT_CONFIG_NOSYSTEM=1 \
        cargo test --locked "${args[@]}" \
        > "$canary/test.log" 2>&1
    cargo_status=$?
    if ! command grep -q '^test result:' "$canary/test.log"; then
        echo "BUILD-FAIL  $t (cargo test exited $cargo_status; log kept at $canary/test.log)"
        failures=$((failures + 1))
        canary=
        continue
    fi
    if [ "$cargo_status" -ne 0 ]; then
        echo "TEST-FAIL  $t (cargo test exited $cargo_status; log kept at $canary/test.log)"
        failures=$((failures + 1))
    fi
    if ! snapshot "$canary" > "$canary/after.txt"; then
        echo "SNAPSHOT-FAIL  $t (the canary could not be listed after the run; kept at $canary)"
        failures=$((failures + 1))
        canary=
        continue
    fi
    changed=
    if ! cmp -s "$canary/before.txt" "$canary/after.txt"; then
        first=$(diff "$canary/before.txt" "$canary/after.txt" | command grep -E '^[<>]' | head -1)
        changed="the canary changed ($first)"
    fi
    git config --file "$canary/gitconfig" --get amont.agent.enabled > /dev/null 2>&1 \
        || changed="${changed:-the old git key amont.agent.enabled is gone}"
    if [ -n "$changed" ]; then
        echo "LEAK  $t: $changed (canary kept at $canary)"
        failures=$((failures + 1))
    elif [ "$cargo_status" -eq 0 ]; then
        echo "ok    $t"
        rm -rf "$canary"
    fi
    canary=
done
if [ "$failures" -gt 0 ]; then
    echo "$failures target(s) changed the machine they ran on, or could not be checked" >&2
    exit 1
fi
