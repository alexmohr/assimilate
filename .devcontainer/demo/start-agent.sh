#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
set -e

AGENT_HOST=$(hostname)
REPO_HOST="demo"
SSH_KEY="/ssh-keys/id_ed25519"

export BORG_RSH="ssh -i $SSH_KEY -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"
export BORG_PASSPHRASE=demo-passphrase-123

echo "==> [$AGENT_HOST] Waiting for SSH key..."
while [ ! -f "$SSH_KEY" ]; do
    sleep 1
done

echo "==> [$AGENT_HOST] Waiting for repos-ready..."
while [ ! -f /seeds/repos-ready ]; do
    sleep 1
done

echo "==> [$AGENT_HOST] Loading tokens..."
. /seeds/tokens.env

case "$AGENT_HOST" in
    web-server-01)  BORG_AGENT_TOKEN="$AGENT_TOKEN_1" ;;
    db-server-01)   BORG_AGENT_TOKEN="$AGENT_TOKEN_2" ;;
    media-store-01) BORG_AGENT_TOKEN="$AGENT_TOKEN_3" ;;
    *)
        echo "ERROR: Unknown agent hostname: $AGENT_HOST" >&2
        exit 1
        ;;
esac
export BORG_AGENT_TOKEN

echo "==> [$AGENT_HOST] Creating archives..."

case "$AGENT_HOST" in
    web-server-01)
        for i in $(seq 1 14); do
            ARCHIVE_DATE=$(date -u -d "$i days ago" +%Y-%m-%dT02:00:00 2>/dev/null || date -u -v-"${i}"d +%Y-%m-%dT02:00:00)
            ARCHIVE_DIR=$(mktemp -d)
            mkdir -p "$ARCHIVE_DIR/var/www/html" "$ARCHIVE_DIR/etc/nginx/conf.d"
            echo "<html><body>Version $i</body></html>" > "$ARCHIVE_DIR/var/www/html/index.html"
            echo "server { listen 80; }" > "$ARCHIVE_DIR/etc/nginx/conf.d/default.conf"
            echo "Restore this file from the archive browser." > "$ARCHIVE_DIR/restore-example.txt"
            dd if=/dev/urandom of="$ARCHIVE_DIR/var/www/html/app.js" bs=1024 count=$((50 + i * 10)) 2>/dev/null
            borg create --lock-wait 60 --timestamp "$ARCHIVE_DATE" \
                "ssh://borg@$REPO_HOST/backup/repos/server-daily::web-server-01-backup-$ARCHIVE_DATE" \
                "$ARCHIVE_DIR"
            rm -rf "$ARCHIVE_DIR"
        done
        ;;
    db-server-01)
        for i in $(seq 1 24); do
            ARCHIVE_DATE=$(date -u -d "$i hours ago" +%Y-%m-%dT%H:00:00 2>/dev/null || date -u -v-"${i}"H +%Y-%m-%dT%H:00:00)
            ARCHIVE_DIR=$(mktemp -d)
            mkdir -p "$ARCHIVE_DIR/tmp" "$ARCHIVE_DIR/var/lib/postgresql"
            echo "-- pg_dump output v$i" > "$ARCHIVE_DIR/tmp/mydb.sql"
            dd if=/dev/urandom of="$ARCHIVE_DIR/var/lib/postgresql/data.bin" bs=1024 count=$((100 + i * 20)) 2>/dev/null
            borg create --lock-wait 60 --timestamp "$ARCHIVE_DATE" \
                "ssh://borg@$REPO_HOST/backup/repos/database-hourly::db-server-01-backup-$ARCHIVE_DATE" \
                "$ARCHIVE_DIR"
            rm -rf "$ARCHIVE_DIR"
        done
        ;;
    media-store-01)
        for i in $(seq 1 6); do
            ARCHIVE_DATE=$(date -u -d "$((i * 7)) days ago" +%Y-%m-%dT03:00:00 2>/dev/null || date -u -v-"$((i * 7))"d +%Y-%m-%dT03:00:00)
            ARCHIVE_DIR=$(mktemp -d)
            mkdir -p "$ARCHIVE_DIR/mnt/media/photos" "$ARCHIVE_DIR/mnt/media/videos"
            dd if=/dev/urandom of="$ARCHIVE_DIR/mnt/media/photos/img_$i.jpg" bs=1024 count=$((200 + i * 50)) 2>/dev/null
            dd if=/dev/urandom of="$ARCHIVE_DIR/mnt/media/videos/clip_$i.mp4" bs=1024 count=$((500 + i * 100)) 2>/dev/null
            borg create --lock-wait 60 --timestamp "$ARCHIVE_DATE" \
                "ssh://borg@$REPO_HOST/backup/repos/media-weekly::media-store-01-backup-$ARCHIVE_DATE" \
                "$ARCHIVE_DIR"
            rm -rf "$ARCHIVE_DIR"
        done
        ;;
esac

echo "==> [$AGENT_HOST] Archives created. Signaling done..."
touch "/seeds/done-$AGENT_HOST"

echo "==> [$AGENT_HOST] Starting agent..."
exec /app/agent
