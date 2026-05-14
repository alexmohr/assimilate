#!/bin/sh

# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
set -e

SSH_DIR="${SSH_KEY_DIR:-/ssh-keys}"
SSH_KEY="$SSH_DIR/id_ed25519"

if [ ! -f "$SSH_KEY" ]; then
    echo "==> Generating SSH key pair for dev..."
    ssh-keygen -t ed25519 -f "$SSH_KEY" -N "" -C "assimilate-dev"
fi

echo "==> SSH public key (add to borg repo host if testing outside devcontainer):"
cat "$SSH_KEY.pub"
echo ""

# Configure local sshd for localhost borg access
echo "==> Configuring local SSH server..."
cp "$SSH_KEY.pub" /home/borg/.ssh/authorized_keys
chmod 600 /home/borg/.ssh/authorized_keys
chown borg:borg /home/borg/.ssh/authorized_keys

# Generate host keys if missing (first run)
if [ ! -f /etc/ssh/ssh_host_ed25519_key ]; then
    ssh-keygen -A
fi

# Start sshd
/usr/sbin/sshd

# Trust our own host key so SSH connections to localhost don't prompt
ssh-keyscan -H localhost >> /etc/ssh/ssh_known_hosts 2>/dev/null

echo "==> Local SSH server running on port 22 (user: borg)"
echo "    Test repo path: /backup/repos/<name>"

# Build agent binary so deploy API can find it as a sibling of the server binary
echo "==> Building agent binary..."
cargo build -p agent
echo "==> Agent binary ready at /workspace/target/debug/agent"

# Start ssh-agent and load the key
eval "$(ssh-agent)" > /dev/null
ssh-add "$SSH_KEY"

# Export for child processes (server needs this for SSH agent forwarding)
echo "export SSH_AUTH_SOCK=$SSH_AUTH_SOCK" > /tmp/ssh-agent-env.sh

echo "==> Dev environment ready"
echo "    Run server:  source /tmp/ssh-agent-env.sh && cargo run -p server"
echo "    Run agent:   BORG_SERVER_URL=http://localhost:8080 BORG_AGENT_TOKEN=<token> cargo run -p agent"
echo "    Run frontend: cd frontend && npm run dev"
echo ""
echo "    Borg repo (localhost): ssh://borg@localhost:22//backup/repos/<name>"
echo "    Borg repo (borg-repo): ssh://borg@borg-repo:22//backup/repos/<name>"
