#!/bin/bash
# Which integration tests would migrate the machine they run on?
#
# Every greenpiste subcommand except `migrate` and `doctor` migrates the
# machine first, so a test that starts the built binary without pointing it at
# a scratch config directory, state home and git config migrates the REAL home
# of whoever runs `cargo test`. This script runs each integration-test binary
# (or the ones named) against a canary old-style machine whose every path is a
# scratch directory, and fails when a test migrated it.
#
#   scripts/leak-hunt.sh                 every tests/*.rs
#   scripts/leak-hunt.sh block shipped   only those
#
# A canary migrated means: the done-marker exists, the old config root became a
# symlink, or the old key `amont.agent.enabled` is gone. A test file whose
# `cargo test` fails to build or run (no `test result:` line) is BUILD-FAIL,
# never ok. See also tests/spawn_isolation.rs, the static check. A test that only
# writes its own `greenpiste.*` keys into the inherited git config is not a
# leak and is not reported.
set -u
root=$(git rev-parse --show-toplevel) || exit 2
cd "$root" || exit 2
real_home=$HOME
if [ "$#" -gt 0 ]; then
    tests="$*"
else
    tests=$(cd tests && command ls ./*.rs | sed 's|^\./||;s|\.rs$||' | sort)
fi
leaks=0
for t in $tests; do
    canary=$(mktemp -d "${TMPDIR:-/tmp}/greenpiste-canary.XXXXXX")
    mkdir -p "$canary/home" "$canary/claude/amont-agent" "$canary/state/amont-agent"
    printf 'amont-agent-v1\n' > "$canary/claude/amont-agent/journal"
    printf '[amont "agent"]\n\tenabled = true\n' > "$canary/gitconfig"
    env HOME="$canary/home" CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}" \
        RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}" \
        CLAUDE_CONFIG_DIR="$canary/claude" XDG_STATE_HOME="$canary/state" \
        XDG_CONFIG_HOME="$canary/xdg" GIT_CONFIG_GLOBAL="$canary/gitconfig" \
        GIT_CONFIG_NOSYSTEM=1 \
        cargo test --locked --features test-hooks --test "$t" --no-fail-fast \
        > "$canary/test.log" 2>&1
    cargo_status=$?
    if [ "$cargo_status" -ne 0 ] || ! command grep -q '^test result:' "$canary/test.log"; then
        echo "BUILD-FAIL  $t (cargo test exited $cargo_status; log kept at $canary/test.log)"
        leaks=$((leaks + 1))
        continue
    fi
    migrated=
    [ -e "$canary/claude/greenpiste-migrated" ] && migrated="the done-marker was written"
    [ -L "$canary/claude/amont-agent" ] && migrated="the old config root became a symlink"
    git config --file "$canary/gitconfig" --get amont.agent.enabled > /dev/null 2>&1 \
        || migrated="${migrated:-the old git key amont.agent.enabled is gone}"
    if [ -n "$migrated" ]; then
        echo "LEAK  $t: $migrated (canary kept at $canary)"
        leaks=$((leaks + 1))
    else
        echo "ok    $t"
        rm -rf "$canary"
    fi
done
if [ "$leaks" -gt 0 ]; then
    echo "$leaks test file(s) would migrate the machine they run on" >&2
    exit 1
fi
