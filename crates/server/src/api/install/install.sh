#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
#
# Installs the Assimilate agent on this host and enrolls it with a server.
#
#   curl -fsSL https://SERVER/install.sh | sudo sh -s -- \
#     --server https://SERVER --token enr_... --server-key KEY
#
# The enrollment token is single-use and short-lived. It is traded, over a
# channel that only a server holding KEY can complete, for the agent's own
# token, which is written where only root can read it.
set -eu

# The release this server was built from, for the hosts that download the
# agent from GitHub instead of from the server.
VERSION="@VERSION@"
RELEASES_URL="${ASSIMILATE_RELEASES_URL:-https://github.com/alexmohr/assimilate/releases/download}"

SERVER=""
TOKEN=""
SERVER_KEY=""
SKIP_SERVICE=0
FORCE=0

LINUX_BIN="/usr/local/bin/assimilate-agent"
LINUX_ENV="/etc/assimilate-agent.env"
LINUX_UNIT="/etc/systemd/system/assimilate-agent.service"
MAC_DIR="/Library/Assimilate"

die() {
    printf 'install.sh: %s\n' "$1" >&2
    exit 1
}

info() {
    printf '==> %s\n' "$1"
}

usage() {
    cat <<USAGE
Usage: install.sh --server URL --token TOKEN --server-key KEY [options]

  --server URL       The Assimilate server, e.g. https://backup.example.com
  --token TOKEN      The single-use enrollment token from the server's Add agent dialog
  --server-key KEY   The server's agent channel key, shown next to the token
  --skip-service     Install and enroll, but do not start the agent at boot
  --force            Install again on a host that already has an agent token
USAGE
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --server) [ "$#" -ge 2 ] || die "--server needs a value"; SERVER="$2"; shift 2 ;;
        --token) [ "$#" -ge 2 ] || die "--token needs a value"; TOKEN="$2"; shift 2 ;;
        --server-key) [ "$#" -ge 2 ] || die "--server-key needs a value"; SERVER_KEY="$2"; shift 2 ;;
        --skip-service) SKIP_SERVICE=1; shift ;;
        --force) FORCE=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) usage >&2; die "unknown option: $1" ;;
    esac
done

[ -n "$SERVER" ] || die "--server is required"
[ -n "$TOKEN" ] || die "--token is required"
[ -n "$SERVER_KEY" ] || die "--server-key is required"
SERVER="${SERVER%/}"
case "$SERVER" in
    https://* | http://*) ;;
    *) die "--server must start with https:// or http://" ;;
esac
[ "$(id -u)" -eq 0 ] || die "run this as root, for example with sudo"
command -v curl >/dev/null 2>&1 || die "curl is required"

case "$(uname -s)" in
    Linux) OS=linux ;;
    Darwin) OS=macos ;;
    *) die "unsupported operating system: $(uname -s); on Windows use install.ps1" ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) ARCH=x86_64 ;;
    aarch64 | arm64) ARCH=aarch64 ;;
    *) die "unsupported architecture: $(uname -m)" ;;
esac
info "Detected $OS on $ARCH"

if [ "$OS" = linux ]; then
    [ -e "$LINUX_ENV" ] && [ "$FORCE" -eq 0 ] \
        && die "$LINUX_ENV exists, so this host already has an agent token; use --force to install again"
else
    [ -e /var/db/assimilate-agent/agent.json ] && [ "$FORCE" -eq 0 ] \
        && die "this host already has an agent; use --force to install again"
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

verify() {
    # verify FILE EXPECTED
    actual="$(sha256_of "$1")"
    [ "$actual" = "$2" ] || die "the SHA-256 of $1 is $actual, expected $2"
    info "SHA-256 verified"
}

download() {
    # download URL FILE
    curl -fsSL --retry 3 -o "$2" "$1" || die "could not download $1"
}

install_linux_binary() {
    info "Downloading the agent from $SERVER"
    download "$SERVER/download/agent?os=linux&arch=$ARCH" "$WORK/agent"
    download "$SERVER/download/agent.sha256?os=linux&arch=$ARCH" "$WORK/agent.sha256"
    verify "$WORK/agent" "$(cut -d ' ' -f 1 <"$WORK/agent.sha256")"
    install -m 755 "$WORK/agent" "$LINUX_BIN"
    AGENT="$LINUX_BIN"
}

install_macos_binary() {
    [ "$VERSION" != "@""VERSION@" ] || die "this script was not served by an Assimilate server"
    if [ "$ARCH" = aarch64 ]; then TARGET=aarch64-apple-darwin; else TARGET=x86_64-apple-darwin; fi
    NAME="assimilate-agent-$VERSION-$TARGET.tar.gz"
    BASE="$RELEASES_URL/v$VERSION"
    info "Downloading $NAME"
    download "$BASE/$NAME" "$WORK/$NAME"
    download "$BASE/SHA256SUMS" "$WORK/SHA256SUMS"
    EXPECTED="$(grep " $NAME\$" "$WORK/SHA256SUMS" | cut -d ' ' -f 1)"
    [ -n "$EXPECTED" ] || die "SHA256SUMS does not list $NAME"
    verify "$WORK/$NAME" "$EXPECTED"
    # The daemon runs as root, so the agent and every folder above it must be
    # owned by root and writable by no one else.
    mkdir -p "$MAC_DIR"
    tar -xzf "$WORK/$NAME" --no-same-owner -C "$MAC_DIR"
    xattr -dr com.apple.quarantine "$MAC_DIR/assimilate-agent" 2>/dev/null || true
    AGENT="$MAC_DIR/assimilate-agent/assimilate-agent"
}

if [ "$OS" = linux ]; then install_linux_binary; else install_macos_binary; fi

info "Enrolling this host with $SERVER"
AGENT_TOKEN="$(BORG_ENROLLMENT_TOKEN="$TOKEN" "$AGENT" --enroll --server-url "$SERVER" --server-key "$SERVER_KEY")" \
    || die "enrollment failed; create a new token in the Add agent dialog and try again"
[ -n "$AGENT_TOKEN" ] || die "the server returned no agent token"

if [ "$OS" = linux ]; then
    umask 077
    printf 'BORG_AGENT_TOKEN=%s\n' "$AGENT_TOKEN" >"$LINUX_ENV"
    chmod 600 "$LINUX_ENV"
    umask 022
    cat >"$LINUX_UNIT" <<UNIT
[Unit]
Description=Assimilate backup agent
After=network.target

[Service]
Environment=BORG_SERVER_URL=$SERVER
Environment=BORG_SERVER_KEY=$SERVER_KEY
EnvironmentFile=$LINUX_ENV
ExecStart=$LINUX_BIN
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
UNIT
    info "Wrote $LINUX_ENV (mode 600) and $LINUX_UNIT"
    if [ "$SKIP_SERVICE" -eq 1 ]; then
        info "Skipping the service start; run: systemctl daemon-reload && systemctl enable --now assimilate-agent"
    elif [ -d /run/systemd/system ]; then
        systemctl daemon-reload
        systemctl enable --now assimilate-agent
        info "The agent is running and starts at boot"
    else
        info "systemd is not running here; start the agent with: $LINUX_BIN"
    fi
else
    if [ "$SKIP_SERVICE" -eq 1 ]; then
        info "Skipping the daemon; the agent token was not stored"
    else
        BORG_SERVER_URL="$SERVER" BORG_SERVER_KEY="$SERVER_KEY" BORG_AGENT_TOKEN="$AGENT_TOKEN" \
            "$AGENT" service install
        info "The agent is running as a launchd daemon and starts at boot"
    fi
fi
