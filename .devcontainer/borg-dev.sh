#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
#
# Run a borg command against a devcontainer demo/dev repo.
# Works from the host or from inside either container.
#
# Usage:
#   .devcontainer/borg-dev.sh <repo> [borg args...]
#
# Repos: server-daily  database-hourly  media-weekly
#
# Examples:
#   .devcontainer/borg-dev.sh server-daily list
#   .devcontainer/borg-dev.sh database-hourly info
#   .devcontainer/borg-dev.sh media-weekly list --short
#   .devcontainer/borg-dev.sh server-daily list ::web-server-01-backup-2026-06-01T02:00:00
set -euo pipefail

KNOWN_REPOS=(server-daily database-hourly media-weekly)
PASSPHRASE="demo-passphrase-123"
REPO_BASE="/backup/repos"

# Dev-mode SSH settings (borg-repo sidecar)
SSH_KEY="/ssh-keys/id_ed25519"
BORG_USER="borg"
BORG_REPO_HOST="borg-repo"
BORG_REPO_PORT="22"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DEMO_COMPOSE="$SCRIPT_DIR/demo/docker-compose.demo.yml"
DEV_COMPOSE="$SCRIPT_DIR/docker-compose.dev.yml"

usage() {
    cat >&2 <<EOF
Usage: $0 <repo> [borg args...]

Repos: ${KNOWN_REPOS[*]}

BORG_REPO is set automatically; use :: to reference archives within it.

Examples:
  $0 server-daily list
  $0 database-hourly info
  $0 media-weekly list --short
  $0 server-daily list ::web-server-01-backup-2026-06-01T02:00:00
EOF
    exit 1
}

[ $# -lt 1 ] && usage

REPO_NAME="$1"; shift

valid=false
for r in "${KNOWN_REPOS[@]}"; do
    [ "$REPO_NAME" = "$r" ] && valid=true && break
done
if [ "$valid" = "false" ]; then
    echo "Error: unknown repo '$REPO_NAME'. Known: ${KNOWN_REPOS[*]}" >&2
    exit 1
fi

run_demo() {
    # Demo: repos live locally inside the demo container, no SSH needed.
    BORG_PASSPHRASE="$PASSPHRASE" BORG_REPO="$REPO_BASE/$REPO_NAME" borg "$@"
}

run_dev() {
    # Dev: repos are on the borg-repo sidecar, accessed over SSH.
    BORG_PASSPHRASE="$PASSPHRASE" \
    BORG_REPO="ssh://${BORG_USER}@${BORG_REPO_HOST}:${BORG_REPO_PORT}/${REPO_BASE}/${REPO_NAME}" \
    BORG_RSH="ssh -i ${SSH_KEY} -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null" \
        borg "$@"
}

if [ -f "/.dockerenv" ] || [ -f "/run/.containerenv" ]; then
    # Inside a container.
    # Demo container: repos are mounted locally.
    # Dev container: repos are behind the borg-repo sidecar.
    if [ -d "$REPO_BASE/$REPO_NAME" ]; then
        run_demo "$@"
    else
        run_dev "$@"
    fi
else
    # On the host.
    # Detect which environment is running and exec into the right service.
    demo_running=false
    dev_running=false

    if docker compose -f "$DEMO_COMPOSE" ps --status running --services 2>/dev/null | grep -q '^demo$'; then
        demo_running=true
    fi
    if docker compose -f "$DEV_COMPOSE" ps --status running --services 2>/dev/null | grep -q '^dev$'; then
        dev_running=true
    fi

    if [ "$demo_running" = "true" ]; then
        docker compose -f "$DEMO_COMPOSE" exec demo \
            env BORG_PASSPHRASE="$PASSPHRASE" BORG_REPO="$REPO_BASE/$REPO_NAME" \
            borg "$@"
    elif [ "$dev_running" = "true" ]; then
        docker compose -f "$DEV_COMPOSE" exec dev \
            env \
                BORG_PASSPHRASE="$PASSPHRASE" \
                BORG_REPO="ssh://${BORG_USER}@${BORG_REPO_HOST}:${BORG_REPO_PORT}/${REPO_BASE}/${REPO_NAME}" \
                BORG_RSH="ssh -i ${SSH_KEY} -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null" \
            borg "$@"
    else
        echo "Error: neither demo nor dev container is running." >&2
        echo "  Demo: .devcontainer/start.sh --demo" >&2
        echo "  Dev:  .devcontainer/start.sh" >&2
        exit 1
    fi
fi
