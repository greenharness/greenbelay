#!/bin/sh
# Install the greenbelay binary. Nothing else.
#
#   curl -fsSL https://raw.githubusercontent.com/greenharness/greenbelay/main/install/install.sh | sh
#
# This script deliberately does NOT wire the guard into Claude Code. It
# downloads a verified binary, puts it on your PATH, and tells you what to run
# next. That restraint is the point: a program that can REFUSE the commands
# your agent runs should not install itself into your settings.json as a side
# effect of you fetching it. `greenbelay install --write` is a separate,
# deliberate act, and it prints the block before it writes anything.
#
# POSIX sh, no bashisms: this has to run wherever curl does.
set -eu

REPO="greenharness/greenbelay"
# `$HOME/.local/bin` by default. `greenbelay install` writes the ABSOLUTE
# path of this binary into settings.json, so the directory matters only for
# whether you can type the command yourself.

RED='\033[31m'; GREEN='\033[32m'; YELLOW='\033[33m'; OFF='\033[0m'
if [ -n "${NO_COLOR:-}" ] || [ ! -t 1 ]; then RED=''; GREEN=''; YELLOW=''; OFF=''; fi

say()  { printf '  %s\n' "$1"; }
ok()   { printf "  ${GREEN}✓${OFF} %s\n" "$1"; }
warn() { printf "  ${YELLOW}!${OFF} %s\n" "$1" >&2; }
die()  { printf "  ${RED}✗${OFF} %s\n" "$1" >&2; exit 1; }

# The settings are GREENBELAY_BIN_DIR and GREENBELAY_VERSION. Their old names,
# AMONT_AGENT_BIN_DIR and AMONT_AGENT_VERSION, still work while the new one is
# unset, with one warning naming the new one.
BIN_DIR="${GREENBELAY_BIN_DIR:-}"
if [ -z "$BIN_DIR" ] && [ -n "${AMONT_AGENT_BIN_DIR:-}" ]; then
    BIN_DIR="$AMONT_AGENT_BIN_DIR"
    warn "AMONT_AGENT_BIN_DIR is the old name: set GREENBELAY_BIN_DIR instead"
fi
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
VERSION="${GREENBELAY_VERSION:-}"
if [ -z "$VERSION" ] && [ -n "${AMONT_AGENT_VERSION:-}" ]; then
    VERSION="$AMONT_AGENT_VERSION"
    warn "AMONT_AGENT_VERSION is the old name: set GREENBELAY_VERSION instead"
fi
VERSION="${VERSION:-latest}"

need() { command -v "$1" > /dev/null 2>&1 || die "$1 is required and was not found"; }

need uname
need tar

# curl or wget, whichever is here.
if command -v curl > /dev/null 2>&1; then
    fetch() { curl -fsSL "$1"; }
    fetch_to() { curl -fsSL "$1" -o "$2"; }
elif command -v wget > /dev/null 2>&1; then
    fetch() { wget -qO- "$1"; }
    fetch_to() { wget -qO "$2" "$1"; }
else
    die "neither curl nor wget is available"
fi

target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Linux)
            # musl when there is no glibc: the static build runs on distros
            # older than whatever the release was built against, which is the
            # usual reason a "linux" binary fails for somebody.
            if ldd --version 2>&1 | grep -qi musl; then libc=musl; else libc=gnu; fi
            case "$arch" in
                x86_64|amd64)  echo "x86_64-unknown-linux-$libc" ;;
                aarch64|arm64) [ "$libc" = "musl" ] && die "no aarch64 musl build yet — build from source with cargo install greenbelay"
                               echo "aarch64-unknown-linux-gnu" ;;
                *) die "unsupported architecture: $arch" ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                x86_64) echo "x86_64-apple-darwin" ;;
                arm64)  echo "aarch64-apple-darwin" ;;
                *) die "unsupported architecture: $arch" ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*)
            # Reachable: this runs under Git Bash, which every Git for Windows
            # install ships. Point at the PowerShell installer rather than the
            # releases page, because there IS a one-liner for this platform.
            die "on Windows, use PowerShell:
    irm https://raw.githubusercontent.com/$REPO/main/install/install.ps1 | iex
  or download the .zip from https://github.com/$REPO/releases"
            ;;
        *) die "unsupported OS: $os" ;;
    esac
}

resolve_version() {
    if [ "$VERSION" != "latest" ]; then
        echo "${VERSION#v}"
        return
    fi
    # The API rather than the /releases/latest redirect, so a rate-limited or
    # offline run fails LOUDLY here instead of downloading a 404 page and
    # handing you a tarball full of HTML.
    tag=$(fetch "https://api.github.com/repos/$REPO/releases/latest" \
        | sed -n 's/.*"tag_name" *: *"\([^"]*\)".*/\1/p' | head -n 1)
    [ -n "$tag" ] || die "could not determine the latest release (rate limited? set GREENBELAY_VERSION=vX.Y.Z)"
    echo "${tag#v}"
}

main() {
    printf '\n  greenbelay installer\n\n'

    t=$(target)
    v=$(resolve_version)
    name="greenbelay-${v}-${t}"
    base="https://github.com/$REPO/releases/download/v${v}"

    say "version:  $v"
    say "platform: $t"
    say "into:     $BIN_DIR"
    printf '\n'

    tmp=$(mktemp -d)
    # Clean up on the way out however we leave, including Ctrl-C.
    trap 'rm -rf "$tmp"' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM

    if [ -n "${GREENBELAY_ARCHIVE_DIR:-}" ]; then
        # A local archive (the release's standalone check): the same checks
        # as a download, read from a directory instead of the release.
        say "from:     $GREENBELAY_ARCHIVE_DIR"
        cp "$GREENBELAY_ARCHIVE_DIR/${name}.tar.gz" "$tmp/${name}.tar.gz" \
            || die "no ${name}.tar.gz in $GREENBELAY_ARCHIVE_DIR"
    else
        say "downloading…"
        fetch_to "$base/${name}.tar.gz" "$tmp/${name}.tar.gz" \
            || die "download failed: $base/${name}.tar.gz"
    fi

    # Checksums are not optional here. This binary runs on every commit with
    # your credentials and reads every staged file; verifying what it is before
    # putting it in that position is the whole argument the project makes about
    # its own dependencies, applied to itself.
    if if [ -n "${GREENBELAY_ARCHIVE_DIR:-}" ]; then
        cp "$GREENBELAY_ARCHIVE_DIR/SHA256SUMS" "$tmp/SHA256SUMS" 2> /dev/null
    else
        fetch_to "$base/SHA256SUMS" "$tmp/SHA256SUMS" 2> /dev/null
    fi; then
        if command -v sha256sum > /dev/null 2>&1; then
            got=$(sha256sum "$tmp/${name}.tar.gz" | cut -d' ' -f1)
        elif command -v shasum > /dev/null 2>&1; then
            got=$(shasum -a 256 "$tmp/${name}.tar.gz" | cut -d' ' -f1)
        else
            got=""
        fi
        if [ -n "$got" ]; then
            want=$(grep " ${name}.tar.gz\$" "$tmp/SHA256SUMS" | cut -d' ' -f1 | head -n 1)
            [ -n "$want" ] || die "SHA256SUMS has no entry for ${name}.tar.gz"
            [ "$got" = "$want" ] || die "checksum mismatch — refusing to install
    expected $want
    got      $got"
            ok "checksum verified"
        else
            warn "no sha256 tool found — the download was NOT verified"
        fi
    else
        warn "no SHA256SUMS published for this release — the download was NOT verified"
    fi

    tar xzf "$tmp/${name}.tar.gz" -C "$tmp"
    # Whether this is an upgrade decides what to say at the end: the
    # first-install steps are wrong advice the second time.
    upgrading=0
    [ -x "$BIN_DIR/greenbelay" ] && upgrading=1
    mkdir -p "$BIN_DIR"
    b=greenbelay
    [ -f "$tmp/$name/$b" ] || die "the archive holds no $b"
    # Write to a temporary name and rename over the destination: replacing
    # a RUNNING binary in place fails on some platforms, and rename is
    # atomic, so a half-copied binary never exists. No old-name binary is
    # placed (ADR-0071).
    cp "$tmp/$name/$b" "$BIN_DIR/.$b.new"
    chmod 755 "$BIN_DIR/.$b.new"
    mv "$BIN_DIR/.$b.new" "$BIN_DIR/$b"
    ok "installed $BIN_DIR/greenbelay"

    printf '\n'
    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *) warn "$BIN_DIR is not on your PATH — add it, or you will not be able to run greenbelay yourself" ;;
    esac

    if [ "$upgrading" -eq 1 ]; then
        printf '  Upgraded. The settings.json entry points at this path, so\n'
        printf '  nothing needs rewiring. Confirm it still fires:\n\n'
        printf '    greenbelay doctor\n\n'
    else
        printf '  Installed, and wired into nothing. The guard does not run until\n'
        printf '  Claude Code is told about it:\n\n'
        printf '    greenbelay install          # print the settings block, write nothing\n'
        printf '    greenbelay install --write  # merge it into ~/.claude/settings.json\n'
        printf '    greenbelay doctor           # installed, runnable, and actually firing?\n\n'
        printf '  Every rule but pipe-to-tail ships as observe, so nothing is\n'
        printf '  refused until you have seen what it would have caught:\n\n'
        printf '    greenbelay status\n\n'
    fi
}

main "$@"
