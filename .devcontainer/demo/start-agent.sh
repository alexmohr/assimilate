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
export BORG_AGENT_TOKEN BORG_SERVER_KEY

# The file as it is "now" for a file history's live comparison
# (docs/archives.md#file-history): web-server-01's nginx.conf at the path its
# archives hold it, edited since the newest backup. seed-demo.sh adds that
# directory to the host's daily schedule, so the agent reads it on request.
write_live_nginx() {
    [ "$AGENT_HOST" = web-server-01 ] || return 0
    mkdir -p /tmp/tmp.web-server-01/etc/nginx
    cat > /tmp/tmp.web-server-01/etc/nginx/nginx.conf <<'NGINX'
user www-data;
worker_processes 4;
pid /run/nginx.pid;

events {
    worker_connections 2048;
}

http {
    sendfile on;
    include /etc/nginx/conf.d/rate-limit.conf;
    tcp_nopush on;
    gzip on;
    include /etc/nginx/conf.d/*.conf;
}
NGINX
}

# Restarted after the seed finished (start-demo.sh's demo-seeded marker): the
# archives below already exist, and creating them again under the same names
# would fail, so the agent starts straight away.
if [ -f /seeds/demo-seeded ] && [ -f "/seeds/done-$AGENT_HOST" ]; then
    echo "==> [$AGENT_HOST] Archives already created, starting agent..."
    write_live_nginx
    exec /app/agent
fi

# Every archive date is counted back from this one reading of the clock. Each
# loop below takes a while (one borg create per pass), and asking `date` afresh
# on every pass let an hour or a day roll over mid-loop: two consecutive passes
# then produced the same name, borg refused the second, and `set -e` killed
# the container before it could signal the seed it was done.
NOW=$(date -u +%s)

# archive_date SECONDS_AGO FORMAT - NOW minus SECONDS_AGO, in UTC. GNU date
# takes `-d @epoch`, BSD/macOS date `-r epoch`.
archive_date() {
    date -u -d "@$((NOW - $1))" "+$2" 2>/dev/null || date -u -r "$((NOW - $1))" "+$2"
}

echo "==> [$AGENT_HOST] Creating archives..."

case "$AGENT_HOST" in
    web-server-01)
        # One fixed source directory rather than a fresh mktemp -d per pass:
        # borg stores the absolute path, so only a fixed one gives a file the
        # same path in every archive - what comparing a file between two
        # archives looks it up by. nginx.conf changes in the two newest
        # archives (i=1 and i=2), so comparing either with an older one shows
        # a real diff. Two rather than one: the dual-target copy below lands
        # later on day 1, so the prune a real Run now triggers (keep_daily
        # over every web-server-01 archive) keeps that copy and drops i=1,
        # while i=2 has its day to itself and always survives. It keeps the
        # tmp.* shape mktemp -d gave it, which archive-browsing.spec.ts
        # walks into.
        ARCHIVE_DIR=/tmp/tmp.web-server-01
        for i in $(seq 1 14); do
            ARCHIVE_DATE=$(archive_date "$((i * 86400))" %Y-%m-%dT02:00:00)
            rm -rf "$ARCHIVE_DIR"
            mkdir -p "$ARCHIVE_DIR/var/www/html" "$ARCHIVE_DIR/etc/nginx/conf.d"
            echo "<html><body>Version $i</body></html>" > "$ARCHIVE_DIR/var/www/html/index.html"
            echo "server { listen 80; }" > "$ARCHIVE_DIR/etc/nginx/conf.d/default.conf"
            if [ "$i" -le 2 ]; then
                WORKERS=4
                HTTP_LINE="    include /etc/nginx/conf.d/rate-limit.conf;"
            else
                WORKERS=auto
                HTTP_LINE="    client_max_body_size 64m;"
            fi
            cat > "$ARCHIVE_DIR/etc/nginx/nginx.conf" <<NGINX
user www-data;
worker_processes $WORKERS;
pid /run/nginx.pid;

events {
    worker_connections 768;
}

http {
    sendfile on;
$HTTP_LINE
    tcp_nopush on;
    include /etc/nginx/conf.d/*.conf;
}
NGINX
            echo "Restore this file from the archive browser." > "$ARCHIVE_DIR/restore-example.txt"
            dd if=/dev/urandom of="$ARCHIVE_DIR/var/www/html/app.js" bs=1024 count=$((50 + i * 10)) 2>/dev/null
            borg create --lock-wait 60 --timestamp "$ARCHIVE_DATE" \
                "ssh://borg@$REPO_HOST:22/backup/repos/server-daily::web-server-01-backup-$ARCHIVE_DATE" \
                "$ARCHIVE_DIR"
            rm -rf "$ARCHIVE_DIR"
        done
        # The same source copied into a second repository, which is what a
        # schedule with more than one target actually does: one occurrence,
        # one archive per repository, the same name in each. Without a real
        # archive under both targets the "Web server dual-target" schedule's
        # Backups tab has nothing to scope between, and its Overview cannot
        # show one repository healthy while the other is not.
        for i in 1 3; do
            ARCHIVE_DATE=$(archive_date "$((i * 86400))" %Y-%m-%dT03:30:00)
            ARCHIVE_DIR=$(mktemp -d)
            mkdir -p "$ARCHIVE_DIR/var/www/html"
            echo "<html><body>Dual-target copy $i</body></html>" > "$ARCHIVE_DIR/var/www/html/index.html"
            dd if=/dev/urandom of="$ARCHIVE_DIR/var/www/html/bundle.js" bs=1024 count=$((80 + i * 10)) 2>/dev/null
            borg create --lock-wait 60 --timestamp "$ARCHIVE_DATE" \
                "ssh://borg@$REPO_HOST:22/backup/repos/server-daily::web-server-01-dual-$ARCHIVE_DATE" \
                "$ARCHIVE_DIR"
            # The older of the two also reached the best-effort target; the
            # newer one is the run that failed against it below.
            if [ "$i" = "3" ]; then
                DUAL_OLD_DATE="$ARCHIVE_DATE"
                borg create --lock-wait 60 --timestamp "$ARCHIVE_DATE" \
                    "ssh://borg@$REPO_HOST:22/backup/repos/media-weekly::web-server-01-dual-$ARCHIVE_DATE" \
                    "$ARCHIVE_DIR"
            else
                DUAL_NEW_DATE="$ARCHIVE_DATE"
            fi
            rm -rf "$ARCHIVE_DIR"
        done
        # seed-demo.sh seeds one report per copy and needs these names exactly.
        # It used to read them back out of the `archives` table, which only has
        # them once a repository sync has imported them - not guaranteed by the
        # point the reports are seeded, and the run where it wasn't took the
        # whole demo down with it. The names are known here, so they are
        # published here.
        cat > /seeds/dual-archives.env <<EOF
DUAL_DAILY_NEW=web-server-01-dual-$DUAL_NEW_DATE
DUAL_DAILY_OLD=web-server-01-dual-$DUAL_OLD_DATE
DUAL_WEEKLY_OLD=web-server-01-dual-$DUAL_OLD_DATE
EOF
        ;;
    db-server-01)
        for i in $(seq 1 24); do
            ARCHIVE_DATE=$(archive_date "$((i * 3600))" %Y-%m-%dT%H:00:00)
            ARCHIVE_DIR=$(mktemp -d)
            mkdir -p "$ARCHIVE_DIR/tmp" "$ARCHIVE_DIR/var/lib/postgresql"
            echo "-- pg_dump output v$i" > "$ARCHIVE_DIR/tmp/mydb.sql"
            dd if=/dev/urandom of="$ARCHIVE_DIR/var/lib/postgresql/data.bin" bs=1024 count=$((100 + i * 20)) 2>/dev/null
            # The newest few also carry database dumps the way a schedule
            # with database dumps writes them: a manifest next to one dump
            # per database, for the archive's Databases view and its
            # restore dialog (docs/database-dumps.md#restore-a-database).
            # They are placeholders, not real dump output; the demo has no
            # database server to restore them into.
            if [ "$i" -le 3 ]; then
                DUMPS="$ARCHIVE_DIR/var/lib/assimilate/database-dumps/schedule-1"
                mkdir -p "$DUMPS/kubernetes/shop/orders-db" "$DUMPS/docker/gitea/db" "$DUMPS/host/postgresql" \
                    "$DUMPS/docker/wiki-db" "$DUMPS/kubernetes/chat/mongo"
                for dump in kubernetes/shop/orders-db/orders.pgdump kubernetes/shop/orders-db/payments.pgdump \
                    docker/gitea/db/gitea.pgdump host/postgresql/inventory.pgdump docker/wiki-db/wiki.sql \
                    kubernetes/chat/mongo/rocket.mongoarchive; do
                    dd if=/dev/urandom of="$DUMPS/$dump" bs=1024 count=$((40 + i * 5)) 2>/dev/null
                done
                cat > "$DUMPS/databases.json" <<'MANIFEST'
{
  "version": 1,
  "dumps": [
    {"runtime": "kubernetes", "namespace": "shop", "name": "orders-db", "kind": "cnpg_cluster", "engine": "postgresql", "pod": "orders-db-2", "database": "orders", "path": "kubernetes/shop/orders-db/orders.pgdump", "format": "pg_custom"},
    {"runtime": "kubernetes", "namespace": "shop", "name": "orders-db", "kind": "cnpg_cluster", "engine": "postgresql", "pod": "orders-db-2", "database": "payments", "path": "kubernetes/shop/orders-db/payments.pgdump", "format": "pg_custom"},
    {"runtime": "docker", "namespace": "gitea", "name": "db", "kind": "container", "engine": "postgresql", "pod": "gitea-db-1", "database": "gitea", "path": "docker/gitea/db/gitea.pgdump", "format": "pg_custom"},
    {"runtime": "host", "namespace": "", "name": "postgresql", "kind": "host", "engine": "postgresql", "pod": "", "database": "inventory", "path": "host/postgresql/inventory.pgdump", "format": "pg_custom"},
    {"runtime": "docker", "namespace": "", "name": "wiki-db", "kind": "container", "engine": "mariadb", "pod": "wiki-db", "database": "wiki", "path": "docker/wiki-db/wiki.sql", "format": "mysql_sql"},
    {"runtime": "kubernetes", "namespace": "chat", "name": "mongo", "kind": "annotated", "engine": "mongodb", "pod": "mongo-0", "database": "rocket", "path": "kubernetes/chat/mongo/rocket.mongoarchive", "format": "mongo_archive"}
  ]
}
MANIFEST
            fi
            borg create --lock-wait 60 --timestamp "$ARCHIVE_DATE" \
                "ssh://borg@$REPO_HOST:22/backup/repos/database-hourly::db-server-01-backup-$ARCHIVE_DATE" \
                "$ARCHIVE_DIR"
            rm -rf "$ARCHIVE_DIR"
        done
        # A couple of archives into the shared server-daily repo too. Without
        # archives from more than one host on a single schedule there is
        # nothing for the Backups tab's host grouping to group.
        #
        # Hours rather than days: the multi-host schedule runs daily, so
        # day-old archives would leave it permanently Overdue and add an
        # attention item the demo never had. The newest must stay inside one
        # cadence.
        for i in 6 30; do
            ARCHIVE_DATE=$(archive_date "$((i * 3600))" %Y-%m-%dT%H:00:00)
            ARCHIVE_DIR=$(mktemp -d)
            mkdir -p "$ARCHIVE_DIR/etc/postgresql"
            echo "shared_buffers = 256MB" > "$ARCHIVE_DIR/etc/postgresql/postgresql.conf"
            dd if=/dev/urandom of="$ARCHIVE_DIR/etc/postgresql/pg_hba.bin" bs=1024 count=$((40 + i * 10)) 2>/dev/null
            borg create --lock-wait 60 --timestamp "$ARCHIVE_DATE" \
                "ssh://borg@$REPO_HOST:22/backup/repos/server-daily::db-server-01-backup-$ARCHIVE_DATE" \
                "$ARCHIVE_DIR"
            rm -rf "$ARCHIVE_DIR"
        done
        ;;
    media-store-01)
        for i in $(seq 1 6); do
            ARCHIVE_DATE=$(archive_date "$((i * 7 * 86400))" %Y-%m-%dT03:00:00)
            ARCHIVE_DIR=$(mktemp -d)
            mkdir -p "$ARCHIVE_DIR/mnt/media/photos" "$ARCHIVE_DIR/mnt/media/videos"
            dd if=/dev/urandom of="$ARCHIVE_DIR/mnt/media/photos/img_$i.jpg" bs=1024 count=$((200 + i * 50)) 2>/dev/null
            dd if=/dev/urandom of="$ARCHIVE_DIR/mnt/media/videos/clip_$i.mp4" bs=1024 count=$((500 + i * 100)) 2>/dev/null
            borg create --lock-wait 60 --timestamp "$ARCHIVE_DATE" \
                "ssh://borg@$REPO_HOST:22/backup/repos/media-weekly::media-store-01-backup-$ARCHIVE_DATE" \
                "$ARCHIVE_DIR"
            rm -rf "$ARCHIVE_DIR"
        done
        # The second host on the shared server-daily schedule - see the
        # db-server-01 branch above.
        for i in 5 29; do
            ARCHIVE_DATE=$(archive_date "$((i * 3600))" %Y-%m-%dT%H:00:00)
            ARCHIVE_DIR=$(mktemp -d)
            mkdir -p "$ARCHIVE_DIR/etc/samba"
            echo "[global] workgroup = DEMO" > "$ARCHIVE_DIR/etc/samba/smb.conf"
            dd if=/dev/urandom of="$ARCHIVE_DIR/etc/samba/shares.bin" bs=1024 count=$((60 + i * 10)) 2>/dev/null
            borg create --lock-wait 60 --timestamp "$ARCHIVE_DATE" \
                "ssh://borg@$REPO_HOST:22/backup/repos/server-daily::media-store-01-backup-$ARCHIVE_DATE" \
                "$ARCHIVE_DIR"
            rm -rf "$ARCHIVE_DIR"
        done
        ;;
esac

echo "==> [$AGENT_HOST] Archives created. Signaling done..."
touch "/seeds/done-$AGENT_HOST"

write_live_nginx

echo "==> [$AGENT_HOST] Starting agent..."
exec /app/agent
