#!/bin/sh

# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

DEMO_FLAG=false
for _arg in "$@"; do
    case "$_arg" in
        --demo) DEMO_FLAG=true ;;
    esac
done

if [ ! -f "/.dockerenv" ] && [ ! -f "/run/.containerenv" ]; then
    if [ "$DEMO_FLAG" = "true" ]; then
        COMPOSE_FILE="$SCRIPT_DIR/demo/docker-compose.demo.yml"
        if echo "$*" | grep -q -- "--clear"; then
            echo "Stopping and removing demo containers + volumes..."
            docker compose -f "$COMPOSE_FILE" down --remove-orphans --volumes
        fi
        echo "Starting demo environment..."
        exec docker compose -f "$COMPOSE_FILE" up --build
    fi

    echo "Stopping existing containers..."
    docker compose -f "$SCRIPT_DIR/docker-compose.dev.yml" down --remove-orphans
    echo "Starting devcontainer services..."
    docker compose -f "$SCRIPT_DIR/docker-compose.dev.yml" up -d --build
    echo "Running inside dev container..."
    exec docker compose -f "$SCRIPT_DIR/docker-compose.dev.yml" exec dev \
        /workspace/.devcontainer/start.sh "$@"
fi

CLEAR_DB=false
for arg in "$@"; do
    case "$arg" in
        --clear) CLEAR_DB=true ;;
    esac
done

SSH_KEY="${SSH_KEY_DIR:-/ssh-keys}/id_ed25519"
if [ ! -f "$SSH_KEY" ]; then
    echo "==> Generating SSH key pair..."
    ssh-keygen -t ed25519 -f "$SSH_KEY" -N "" -C "assimilate-dev"
fi

cp "$SSH_KEY.pub" /home/borg/.ssh/authorized_keys
chmod 600 /home/borg/.ssh/authorized_keys
chown borg:borg /home/borg/.ssh/authorized_keys

if [ ! -f /etc/ssh/ssh_host_ed25519_key ]; then
    ssh-keygen -A
fi
/usr/sbin/sshd
ssh-keyscan -H localhost >> /etc/ssh/ssh_known_hosts 2>/dev/null

eval "$(ssh-agent -s)"
ssh-add "$SSH_KEY"

echo "==> Waiting for PostgreSQL..."
until PGPASSWORD=borg_dev psql -h postgres -U borg -d borg -c "SELECT 1" > /dev/null 2>&1; do
    sleep 1
done

if [ "$CLEAR_DB" = "true" ]; then
    echo "==> Resetting database (--clear)..."
    PGPASSWORD=borg_dev psql -h postgres -U borg -d borg -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
fi

cd /workspace/frontend
if [ ! -f "node_modules/.package-lock.json" ] || ! diff -q package-lock.json node_modules/.package-lock.json > /dev/null 2>&1; then
    npm ci
fi
echo "==> Building frontend..."
npm run build

cd /workspace

echo "==> Building agent binary..."
cargo build -p agent

echo "==> Starting server..."
export ASSIMILATE_STATIC_DIR=/workspace/frontend/dist
cargo run -p server &
SERVER_PID=$!

trap 'kill '"$SERVER_PID"' 2>/dev/null; ssh-agent -k > /dev/null 2>&1' EXIT INT TERM

echo ""
echo "App: http://localhost:8080"
echo ""
echo "Borg repo (localhost):  ssh://borg@localhost:22//backup/repos/<name>"
echo "Borg repo (borg-repo):  ssh://borg@borg-repo:22//backup/repos/<name>"
echo ""
echo "To start the agent, open a new terminal and run:"
echo "  docker compose -f .devcontainer/docker-compose.dev.yml exec dev bash -c 'eval \$(ssh-agent -s) && ssh-add /ssh-keys/id_ed25519 && BORG_SERVER_URL=http://localhost:8080 BORG_AGENT_TOKEN=<token> cargo run -p agent'"
echo ""

wait
