#!/bin/sh

# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
set -e

AUTH_KEYS="/home/borg/.ssh/authorized_keys"
SSH_KEYS_DIR="/ssh-keys"

echo "==> Waiting for dev container SSH public key..."
while [ ! -f "$SSH_KEYS_DIR/id_ed25519.pub" ]; do
    sleep 1
done

cp "$SSH_KEYS_DIR/id_ed25519.pub" "$AUTH_KEYS"
chmod 600 "$AUTH_KEYS"
chown borg:borg "$AUTH_KEYS"

echo "==> Authorized key installed"
echo "==> Starting sshd"

exec /usr/sbin/sshd -D -e
