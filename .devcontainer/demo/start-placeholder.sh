#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
set -e

PLACEHOLDER_HOST=$(hostname)
REPO_HOST="demo"
SSH_KEY="/ssh-keys/id_ed25519"

export BORG_RSH="ssh -i $SSH_KEY -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"
export BORG_PASSPHRASE=demo-passphrase-123

echo "==> [$PLACEHOLDER_HOST] Waiting for SSH key..."
while [ ! -f "$SSH_KEY" ]; do
    sleep 1
done

echo "==> [$PLACEHOLDER_HOST] Waiting for repos-ready..."
while [ ! -f /seeds/repos-ready ]; do
    sleep 1
done

UNMATCHED_DATE=$(date -u -d "5 days ago" +%Y-%m-%dT04:00:00 2>/dev/null || date -u -v-5d +%Y-%m-%dT04:00:00)

case "$PLACEHOLDER_HOST" in
    old-webserver)
        REPO="server-daily"
        ;;
    legacy-db-prod)
        REPO="database-hourly"
        ;;
    *)
        echo "ERROR: Unknown placeholder hostname: $PLACEHOLDER_HOST" >&2
        exit 1
        ;;
esac

echo "==> [$PLACEHOLDER_HOST] Creating unmatched archive in $REPO..."
ARCHIVE_DIR=$(mktemp -d)
mkdir -p "$ARCHIVE_DIR/tmp"
echo "old backup data" > "$ARCHIVE_DIR/tmp/data.txt"
borg create --lock-wait 60 --timestamp "$UNMATCHED_DATE" \
    "ssh://borg@$REPO_HOST/backup/repos/$REPO::${PLACEHOLDER_HOST}-backup-$UNMATCHED_DATE" \
    "$ARCHIVE_DIR"
rm -rf "$ARCHIVE_DIR"

echo "==> [$PLACEHOLDER_HOST] Done. Signaling..."
touch "/seeds/done-$PLACEHOLDER_HOST"
