#!/bin/sh

# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
set -e

SSH_DIR="${SSH_KEY_DIR:-/app/ssh}"
SSH_KEY="$SSH_DIR/id_ed25519"
SSH_PUB="$SSH_KEY.pub"

# Fix volume ownership (may be root-owned from previous versions)
chown -R assimilate:assimilate "$SSH_DIR"

# Generate key pair on first run
if [ ! -f "$SSH_KEY" ]; then
    gosu assimilate ssh-keygen -t ed25519 -f "$SSH_KEY" -N "" -C "assimilate-server"
    echo "==> Generated new SSH key pair in $SSH_DIR"
fi

echo "==> SSH public key:"
cat "$SSH_PUB"
echo ""

# Start ssh-agent and load the key
eval "$(gosu assimilate ssh-agent -a /tmp/ssh-agent.sock)" > /dev/null
gosu assimilate ssh-add "$SSH_KEY"

export SSH_AUTH_SOCK=/tmp/ssh-agent.sock

exec gosu assimilate /app/server "$@"
