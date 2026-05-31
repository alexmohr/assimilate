#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
set -e

BASE_URL="http://localhost:8080"

login() {
    COOKIE=$(curl -sf -D - -X POST "$BASE_URL/api/auth/login" \
        -H "Content-Type: application/json" \
        -d '{"username":"admin","password":"admin"}' | grep -i set-cookie | head -1 | sed 's/.*: //' | cut -d';' -f1)
    AUTH_HEADER="Cookie: $COOKIE"
}

api() {
    METHOD="$1"; shift
    PATH_="$1"; shift
    if [ $# -gt 0 ]; then
        curl -sf -X "$METHOD" "$BASE_URL$PATH_" -H "Content-Type: application/json" -H "$AUTH_HEADER" -d "$1"
    else
        curl -sf -X "$METHOD" "$BASE_URL$PATH_" -H "$AUTH_HEADER"
    fi
}

echo "==> Creating borg repositories on disk..."
for REPO_NAME in server-daily database-hourly media-weekly; do
    REPO_DIR="/backup/repos/$REPO_NAME"
    if [ ! -d "$REPO_DIR" ]; then
        su -c "BORG_PASSPHRASE=demo-passphrase-123 borg init --encryption=repokey-blake2 $REPO_DIR" borg
    fi
done

echo "==> Cleaning up existing demo data (idempotent re-run)..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<'SQL' > /dev/null 2>&1
DELETE FROM backup_reports WHERE client_id IN (SELECT id FROM clients WHERE hostname IN ('web-server-01','db-server-01','media-store-01','old-webserver (imported)','legacy-db-prod (imported)'));
DELETE FROM schedules WHERE id IN (SELECT st.schedule_id FROM schedule_targets st JOIN clients c ON c.id = st.client_id WHERE c.hostname IN ('web-server-01','db-server-01','media-store-01'));
DELETE FROM ssh_tunnels WHERE client_id IN (SELECT id FROM clients WHERE hostname IN ('web-server-01','db-server-01','media-store-01'));
DELETE FROM client_hostname_patterns WHERE client_id IN (SELECT id FROM clients WHERE hostname IN ('web-server-01','db-server-01','media-store-01'));
DELETE FROM clients WHERE hostname IN ('web-server-01','db-server-01','media-store-01','old-webserver (imported)','legacy-db-prod (imported)');
DELETE FROM repo_quotas WHERE repo_id IN (SELECT id FROM repos WHERE name IN ('server-daily','database-hourly','media-weekly'));
DELETE FROM archive_tags WHERE repo_id IN (SELECT id FROM repos WHERE name IN ('server-daily','database-hourly','media-weekly'));
DELETE FROM notification_rules;
DELETE FROM notification_channels;
DELETE FROM repos WHERE name IN ('server-daily','database-hourly','media-weekly');
DELETE FROM system_events;
DELETE FROM audit_log;
DELETE FROM login_attempts;
DELETE FROM users WHERE username IN ('operator1','viewer1');
-- Reset admin password to 'admin' (bcrypt cost 10, pre-computed)
UPDATE users SET password_hash = '$2b$10$HvauZloS2N8QIfViDXmtp.rpWOawMeLdgWdBQQDHl3jD7Mhw7C3/e', must_change_password = false WHERE username = 'admin';
INSERT INTO users (username, password_hash, role, must_change_password)
VALUES ('admin', '$2b$10$HvauZloS2N8QIfViDXmtp.rpWOawMeLdgWdBQQDHl3jD7Mhw7C3/e', 'admin', false)
ON CONFLICT (username) DO NOTHING;
SQL

echo "==> Logging in..."
login

echo "==> Setting timezone to Europe/Berlin..."
api PUT /api/system/settings '{"timezone":"Europe/Berlin","retention_days":7}'

echo "==> Registering hosts..."
WEB01_TOKEN=$(api POST "/api/clients" '{"hostname":"web-server-01","display_name":"Production Web Server"}' | jq -r '.token')
DB01_TOKEN=$(api POST "/api/clients" '{"hostname":"db-server-01","display_name":"Primary Database"}' | jq -r '.token')
MEDIA_TOKEN=$(api POST "/api/clients" '{"hostname":"media-store-01","display_name":"Media Storage NAS"}' | jq -r '.token')

export AGENT_TOKEN_1="$WEB01_TOKEN"
export AGENT_TOKEN_2="$DB01_TOKEN"
export AGENT_TOKEN_3="$MEDIA_TOKEN"

echo "==> Registering repositories..."
api POST "/api/repos" "{
    \"name\": \"server-daily\",
    \"repo_path\": \"/backup/repos/server-daily\",
    \"ssh_user\": \"borg\",
    \"ssh_host\": \"localhost\",
    \"ssh_port\": 22,
    \"passphrase\": \"demo-passphrase-123\",
    \"compression\": \"lz4\"
}" > /dev/null

api POST "/api/repos" "{
    \"name\": \"database-hourly\",
    \"repo_path\": \"/backup/repos/database-hourly\",
    \"ssh_user\": \"borg\",
    \"ssh_host\": \"localhost\",
    \"ssh_port\": 22,
    \"passphrase\": \"demo-passphrase-123\",
    \"compression\": \"zstd,3\"
}" > /dev/null

api POST "/api/repos" "{
    \"name\": \"media-weekly\",
    \"repo_path\": \"/backup/repos/media-weekly\",
    \"ssh_user\": \"borg\",
    \"ssh_host\": \"localhost\",
    \"ssh_port\": 22,
    \"passphrase\": \"demo-passphrase-123\",
    \"compression\": \"lz4\"
}" > /dev/null

echo "==> Creating sample borg archives for browsing/diff..."

# Helper: create borg archive with spoofed hostname metadata.
# Borg 1.x stores socket.gethostname() in archive metadata with no env override,
# so we monkey-patch it via Python before invoking borg's main().
# Usage: borg_create_as <hostname> <repo::archive> <source_path>
borg_create_as() {
    local fake_host="$1" repo_archive="$2" source_path="$3"
    su -c "cd $source_path && BORG_PASSPHRASE=demo-passphrase-123 python3 -c \"
import socket, platform, sys
socket.gethostname = lambda: '$fake_host'
platform.node = lambda: '$fake_host'
from borg.archiver import main
sys.argv = ['borg', 'create', '$repo_archive', '.']
sys.exit(main())
\"" borg
}

for i in 1 2 3; do
    ARCHIVE_DATE=$(date -u -d "$i days ago" +%Y-%m-%dT%H:%M:%S 2>/dev/null || date -u -v-"${i}"d +%Y-%m-%dT%H:%M:%S)
    ARCHIVE_DIR=$(mktemp -d)
    chmod 755 "$ARCHIVE_DIR"
    mkdir -p "$ARCHIVE_DIR/var/www/html" "$ARCHIVE_DIR/etc/nginx/conf.d"
    echo "<html><body>Version $i</body></html>" > "$ARCHIVE_DIR/var/www/html/index.html"
    echo "server { listen 80; }" > "$ARCHIVE_DIR/etc/nginx/conf.d/default.conf"
    dd if=/dev/urandom of="$ARCHIVE_DIR/var/www/html/app.js" bs=1024 count=$((50 + i * 10)) 2>/dev/null
    borg_create_as "web-server-01" "/backup/repos/server-daily::web-server-01-backup-$ARCHIVE_DATE" "$ARCHIVE_DIR"
    rm -rf "$ARCHIVE_DIR"
done

for i in 1 2; do
    ARCHIVE_DATE=$(date -u -d "$i hours ago" +%Y-%m-%dT%H:%M:%S 2>/dev/null || date -u -v-"${i}"H +%Y-%m-%dT%H:%M:%S)
    ARCHIVE_DIR=$(mktemp -d)
    chmod 755 "$ARCHIVE_DIR"
    mkdir -p "$ARCHIVE_DIR/tmp" "$ARCHIVE_DIR/var/lib/postgresql"
    echo "-- pg_dump output v$i" > "$ARCHIVE_DIR/tmp/mydb.sql"
    dd if=/dev/urandom of="$ARCHIVE_DIR/var/lib/postgresql/data.bin" bs=1024 count=$((100 + i * 20)) 2>/dev/null
    borg_create_as "db-server-01" "/backup/repos/database-hourly::db-server-01-backup-$ARCHIVE_DATE" "$ARCHIVE_DIR"
    rm -rf "$ARCHIVE_DIR"
done

for i in 1 2; do
    ARCHIVE_DATE=$(date -u -d "$((i + 3)) days ago" +%Y-%m-%dT%H:%M:%S 2>/dev/null || date -u -v-"$((i + 3))"d +%Y-%m-%dT%H:%M:%S)
    ARCHIVE_DIR=$(mktemp -d)
    chmod 755 "$ARCHIVE_DIR"
    mkdir -p "$ARCHIVE_DIR/mnt/media/photos" "$ARCHIVE_DIR/mnt/media/videos"
    dd if=/dev/urandom of="$ARCHIVE_DIR/mnt/media/photos/img_$i.jpg" bs=1024 count=$((200 + i * 50)) 2>/dev/null
    dd if=/dev/urandom of="$ARCHIVE_DIR/mnt/media/videos/clip_$i.mp4" bs=1024 count=$((500 + i * 100)) 2>/dev/null
    borg_create_as "media-store-01" "/backup/repos/media-weekly::media-store-01-backup-$ARCHIVE_DATE" "$ARCHIVE_DIR"
    rm -rf "$ARCHIVE_DIR"
done

echo "==> Creating unmatched archives (unknown hostnames)..."
UNMATCHED_DATE=$(date -u -d "5 days ago" +%Y-%m-%dT%H:%M:%S 2>/dev/null || date -u -v-5d +%Y-%m-%dT%H:%M:%S)
for repo in server-daily database-hourly; do
    ARCHIVE_DIR=$(mktemp -d)
    chmod 755 "$ARCHIVE_DIR"
    mkdir -p "$ARCHIVE_DIR/tmp"
    echo "old backup data" > "$ARCHIVE_DIR/tmp/data.txt"
    borg_create_as "unknown-legacy-host" "/backup/repos/$repo::unknown-host-backup-$UNMATCHED_DATE" "$ARCHIVE_DIR"
    rm -rf "$ARCHIVE_DIR"
done

echo "==> Waiting for repo import to settle..."
sleep 5

echo "==> Syncing repos to import borg archives..."
api POST /api/repos/1/sync > /dev/null
api POST /api/repos/2/sync > /dev/null
api POST /api/repos/3/sync > /dev/null

echo "==> Fetching IDs..."
WEB01_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM clients WHERE hostname='web-server-01'")
DB01_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM clients WHERE hostname='db-server-01'")
MEDIA_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM clients WHERE hostname='media-store-01'")
REPO_DAILY_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM repos WHERE name='server-daily'")
REPO_HOURLY_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM repos WHERE name='database-hourly'")
REPO_WEEKLY_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM repos WHERE name='media-weekly'")

echo "==> Creating schedules..."
api POST "/api/schedules" "{
    \"client_ids\": [$WEB01_ID],
    \"repo_id\": $REPO_DAILY_ID,
    \"cron_expression\": \"0 2 * * *\",
    \"enabled\": true,
    \"keep_daily\": 7,
    \"keep_weekly\": 4,
    \"keep_monthly\": 6,
    \"backup_sources\": [\"/var/www\", \"/etc/nginx\"]
}" > /dev/null

api POST "/api/schedules" "{
    \"client_ids\": [$DB01_ID],
    \"repo_id\": $REPO_HOURLY_ID,
    \"cron_expression\": \"0 * * * *\",
    \"enabled\": true,
    \"keep_daily\": 14,
    \"keep_weekly\": 8,
    \"keep_monthly\": 12,
    \"pre_backup_commands\": [\"pg_dump -U postgres mydb > /tmp/mydb.sql\"],
    \"backup_sources\": [\"/tmp/mydb.sql\", \"/var/lib/postgresql\"],
    \"rate_limit_kbps\": 5000
}" > /dev/null

api POST "/api/schedules" "{
    \"client_ids\": [$MEDIA_ID],
    \"repo_id\": $REPO_WEEKLY_ID,
    \"cron_expression\": \"0 3 * * 0\",
    \"enabled\": true,
    \"keep_daily\": 0,
    \"keep_weekly\": 4,
    \"keep_monthly\": 12,
    \"keep_yearly\": 2,
    \"backup_sources\": [\"/mnt/media/photos\", \"/mnt/media/videos\"]
}" > /dev/null

api POST "/api/schedules" "{
    \"client_ids\": [$WEB01_ID, $DB01_ID, $MEDIA_ID],
    \"repo_id\": $REPO_DAILY_ID,
    \"cron_expression\": \"0 4 * * *\",
    \"enabled\": true,
    \"execution_mode\": \"sequential\",
    \"on_failure\": \"stop\",
    \"keep_daily\": 7,
    \"keep_weekly\": 4,
    \"keep_monthly\": 6,
    \"backup_sources\": [\"/etc\"],
    \"backup_sources_per_host\": [
        {\"client_id\": $WEB01_ID, \"paths\": [\"/var/www\", \"/etc/nginx\", \"/var/log/nginx\"]},
        {\"client_id\": $DB01_ID, \"paths\": [\"/var/lib/postgresql\", \"/etc/postgresql\"]},
        {\"client_id\": $MEDIA_ID, \"paths\": [\"/mnt/media/photos\", \"/mnt/media/videos\"]}
    ]
}" > /dev/null

echo "==> Adding global excludes..."
for PATTERN in "pp:__pycache__" "pp:.cache" "pp:node_modules" "*.pyc" "*.swp" "*~" "/proc" "/sys" "/tmp"; do
    api POST "/api/excludes" "{\"pattern\": \"$PATTERN\"}" > /dev/null 2>&1 || true
done

echo "==> Creating tags..."
api POST "/api/tags" '{"name":"production","color":"#ef4444","scope":"host"}' > /dev/null 2>&1 || true
api POST "/api/tags" '{"name":"staging","color":"#f59e0b","scope":"host"}' > /dev/null 2>&1 || true
api POST "/api/tags" '{"name":"critical","color":"#dc2626","scope":"repo"}' > /dev/null 2>&1 || true
api POST "/api/tags" '{"name":"archival","color":"#6366f1","scope":"repo"}' > /dev/null 2>&1 || true

echo "==> Creating additional users and roles..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<'SQL'
INSERT INTO users (username, password_hash, role) VALUES
    ('operator1', '$2b$12$LJ3m4sFQH/0.s3VDlIBNOeRbEEziXlg5V5X1A0x0aM0ABs3LHfMwq', 'operator'),
    ('viewer1', '$2b$12$LJ3m4sFQH/0.s3VDlIBNOeRbEEziXlg5V5X1A0x0aM0ABs3LHfMwq', 'viewer')
ON CONFLICT (username) DO NOTHING;
SQL

echo "==> Creating groups..."
api POST "/api/groups" '{"name":"backend-team","description":"Backend infrastructure engineers"}' > /dev/null 2>&1 || true
api POST "/api/groups" '{"name":"data-team","description":"Database and analytics team"}' > /dev/null 2>&1 || true

echo "==> Adding hostname aliases for unmatched archive scenario..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO clients (hostname, display_name, agent_token_hash)
VALUES
    ('old-webserver (imported)', NULL, 'imported:no-auth'),
    ('legacy-db-prod (imported)', NULL, 'imported:no-auth')
ON CONFLICT (hostname) DO NOTHING;

-- Mark one imported client as hidden to demonstrate the hide feature
UPDATE clients SET is_hidden = true WHERE hostname = 'legacy-db-prod (imported)';
SQL

PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO client_hostname_patterns (client_id, pattern)
SELECT id, 'web-server-*' FROM clients WHERE hostname='web-server-01'
ON CONFLICT DO NOTHING;
SQL

echo "==> Inserting backup report history..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO backup_reports (client_id, repo_id, started_at, finished_at, status, original_size, compressed_size, deduplicated_size, files_processed, duration_secs, error_message, warnings, borg_version, matched)
SELECT
    $WEB01_ID, $REPO_DAILY_ID,
    NOW() - (n || ' days')::interval - interval '2 hours',
    NOW() - (n || ' days')::interval - interval '2 hours' + (120 + (random() * 180)::int || ' seconds')::interval,
    CASE
        WHEN n = 3 THEN 'warning'
        WHEN n = 7 THEN 'failed'
        ELSE 'success'
    END,
    (1024*1024*1024 * (2.1 + random() * 0.5))::bigint,
    (1024*1024*1024 * (1.5 + random() * 0.3))::bigint,
    (1024*1024*512 * (0.8 + random() * 0.2))::bigint,
    (45000 + (random() * 10000)::int),
    (120 + (random() * 180)::int),
    CASE WHEN n = 7 THEN 'Repository lock could not be acquired after 600s' ELSE NULL END,
    CASE WHEN n = 3 THEN ARRAY['file changed while reading: /var/www/app/cache/sess_abc123'] ELSE ARRAY[]::text[] END,
    '1.4.0',
    true
FROM generate_series(0, 29) AS n;

INSERT INTO backup_reports (client_id, repo_id, started_at, finished_at, status, original_size, compressed_size, deduplicated_size, files_processed, duration_secs, borg_version, matched)
SELECT
    $DB01_ID, $REPO_HOURLY_ID,
    NOW() - (n || ' hours')::interval,
    NOW() - (n || ' hours')::interval + (30 + (random() * 60)::int || ' seconds')::interval,
    CASE WHEN n = 18 THEN 'failed' ELSE 'success' END,
    (1024*1024*256 * (1.0 + random() * 0.3))::bigint,
    (1024*1024*128 * (0.8 + random() * 0.2))::bigint,
    (1024*1024*32 * (0.5 + random() * 0.3))::bigint,
    (200 + (random() * 100)::int),
    (30 + (random() * 60)::int),
    '1.4.0',
    true
FROM generate_series(0, 71) AS n;

INSERT INTO backup_reports (client_id, repo_id, started_at, finished_at, status, original_size, compressed_size, deduplicated_size, files_processed, duration_secs, borg_version, matched)
SELECT
    $MEDIA_ID, $REPO_WEEKLY_ID,
    NOW() - (n * 7 || ' days')::interval - interval '3 hours',
    NOW() - (n * 7 || ' days')::interval - interval '3 hours' + (1800 + (random() * 1200)::int || ' seconds')::interval,
    'success',
    (1024*1024*1024 * (50.0 + random() * 10.0))::bigint,
    (1024*1024*1024 * (48.0 + random() * 8.0))::bigint,
    (1024*1024*1024 * (5.0 + random() * 2.0))::bigint,
    (150000 + (random() * 50000)::int),
    (1800 + (random() * 1200)::int),
    '1.4.0',
    true
FROM generate_series(0, 11) AS n;

INSERT INTO backup_reports (client_id, repo_id, started_at, finished_at, status, original_size, compressed_size, deduplicated_size, files_processed, duration_secs, borg_version, matched)
SELECT
    c.id, $REPO_DAILY_ID,
    NOW() - (n || ' days')::interval - interval '2 hours',
    NOW() - (n || ' days')::interval - interval '2 hours' + interval '90 seconds',
    'success',
    (1024*1024*512)::bigint,
    (1024*1024*400)::bigint,
    (1024*1024*100)::bigint,
    5000, 90, '1.4.0',
    false
FROM clients c, generate_series(0, 14) AS n
WHERE c.hostname = 'old-webserver (imported)';
SQL

echo "==> Linking backup reports to actual borg archives..."
# Get real archive names from borg and update matching report rows
DAILY_ARCHIVES=$(su -c "BORG_PASSPHRASE=demo-passphrase-123 borg list --short /backup/repos/server-daily" borg | grep '^web-server-01-backup-' | sort -r)
HOURLY_ARCHIVES=$(su -c "BORG_PASSPHRASE=demo-passphrase-123 borg list --short /backup/repos/database-hourly" borg | grep '^db-server-01-backup-' | sort -r)

# For each real daily archive, update the report row closest to that archive's date
i=1
for archive in $DAILY_ARCHIVES; do
    PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -c "
        UPDATE backup_reports SET archive_name = '$archive'
        WHERE id = (
            SELECT id FROM backup_reports
            WHERE client_id = $WEB01_ID AND repo_id = $REPO_DAILY_ID AND status = 'success' AND archive_name IS NULL
            ORDER BY started_at DESC
            LIMIT 1 OFFSET $((i - 1))
        );
    " > /dev/null
    i=$((i + 1))
done

# For each real hourly archive, update the report row closest to that archive's date
i=1
for archive in $HOURLY_ARCHIVES; do
    PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -c "
        UPDATE backup_reports SET archive_name = '$archive'
        WHERE id = (
            SELECT id FROM backup_reports
            WHERE client_id = $DB01_ID AND repo_id = $REPO_HOURLY_ID AND status = 'success' AND archive_name IS NULL
            ORDER BY started_at DESC
            LIMIT 1 OFFSET $((i - 1))
        );
    " > /dev/null
    i=$((i + 1))
done

echo "==> Setting up repo quotas..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO repo_quotas (repo_id, warn_bytes, critical_bytes, enabled) VALUES
    ($REPO_DAILY_ID, 10737418240, 16106127360, true),
    ($REPO_HOURLY_ID, 5368709120, 8589934592, true)
ON CONFLICT (repo_id) DO NOTHING;
SQL

echo "==> Adding system events..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO system_events (created_at, event_type, hostname, message) VALUES
    (NOW() - interval '5 minutes', 'agent_connected', 'web-server-01', 'Agent connected (version 0.5.2)'),
    (NOW() - interval '4 minutes', 'agent_connected', 'db-server-01', 'Agent connected (version 0.5.2)'),
    (NOW() - interval '3 minutes', 'agent_connected', 'media-store-01', 'Agent connected (version 0.5.1)'),
    (NOW() - interval '2 days', 'agent_disconnected', 'media-store-01', 'Agent disconnected: connection timeout'),
    (NOW() - interval '7 days', 'backup_failed', 'web-server-01', 'Backup failed: Repository lock could not be acquired'),
    (NOW() - interval '1 day', 'backup_warning', 'web-server-01', 'Backup completed with warnings');
SQL

echo "==> Adding audit log entries..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO audit_log (user_id, username, action, target_type, target_id, details, ip_address, created_at) VALUES
    (1, 'admin', 'repo.create', 'repository', $REPO_DAILY_ID, '{"name":"server-daily"}', '192.168.1.10', NOW() - interval '30 days'),
    (1, 'admin', 'repo.create', 'repository', $REPO_HOURLY_ID, '{"name":"database-hourly"}', '192.168.1.10', NOW() - interval '30 days'),
    (1, 'admin', 'repo.create', 'repository', $REPO_WEEKLY_ID, '{"name":"media-weekly"}', '192.168.1.10', NOW() - interval '29 days'),
    (1, 'admin', 'host.create', 'host', $WEB01_ID, '{"hostname":"web-server-01"}', '192.168.1.10', NOW() - interval '28 days'),
    (1, 'admin', 'host.create', 'host', $DB01_ID, '{"hostname":"db-server-01"}', '192.168.1.10', NOW() - interval '28 days'),
    (1, 'admin', 'host.create', 'host', $MEDIA_ID, '{"hostname":"media-store-01"}', '192.168.1.10', NOW() - interval '27 days'),
    (1, 'admin', 'schedule.create', 'schedule', 1, '{"cron":"0 2 * * *"}', '192.168.1.10', NOW() - interval '27 days'),
    (1, 'admin', 'schedule.create', 'schedule', 2, '{"cron":"0 * * * *"}', '192.168.1.10', NOW() - interval '27 days'),
    (1, 'admin', 'schedule.create', 'schedule', 3, '{"cron":"0 3 * * 0"}', '192.168.1.10', NOW() - interval '26 days'),
    (1, 'admin', 'user.create', 'user', 2, '{"username":"operator1","role":"operator"}', '192.168.1.10', NOW() - interval '25 days'),
    (1, 'admin', 'auth.login', NULL, NULL, NULL, '192.168.1.10', NOW() - interval '1 hour'),
    (1, 'admin', 'quota.configure', 'repository', $REPO_DAILY_ID, '{"warn_gb":10,"critical_gb":15}', '192.168.1.10', NOW() - interval '20 days');
SQL

echo "==> Adding notification channels and rules..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO notification_channels (name, channel_type, config, enabled) VALUES
    ('Ops Webhook', 'webhook', '{"url":"https://hooks.example.com/assimilate","headers":{"Authorization":"Bearer demo-token"}}', true),
    ('Admin Email', 'email', '{"smtp_host":"smtp.example.com","smtp_port":587,"security":"starttls","from":"backups@example.com","to":["admin@example.com"]}', true);

INSERT INTO notification_rules (channel_id, event_type, enabled)
SELECT c.id, e.event_type, true
FROM notification_channels c,
     (VALUES ('backup_failed'), ('backup_warning'), ('agent_disconnected')) AS e(event_type)
WHERE c.name = 'Ops Webhook';

INSERT INTO notification_rules (channel_id, event_type, enabled)
SELECT c.id, e.event_type, true
FROM notification_channels c,
     (VALUES ('backup_failed'), ('backup_success'), ('agent_connected'), ('agent_disconnected')) AS e(event_type)
WHERE c.name = 'Admin Email';
SQL

echo "==> Adding SSH tunnel entry..."
api POST "/api/tunnels" "{\"client_id\":$MEDIA_ID,\"ssh_host\":\"127.0.0.1\",\"ssh_user\":\"borg\",\"ssh_port\":22,\"tunnel_port\":18080,\"enabled\":true}" > /dev/null

echo "==> Adding archive tags..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO archive_tags (repo_id, archive_name, tag, created_by) VALUES
    ($REPO_DAILY_ID, (SELECT name FROM (SELECT 'web-server-01-backup-' || to_char(NOW() - interval '1 day', 'YYYY-MM-DD"T"HH24:MI:SS') AS name) x), 'pre-upgrade', 1),
    ($REPO_DAILY_ID, (SELECT name FROM (SELECT 'web-server-01-backup-' || to_char(NOW() - interval '3 days', 'YYYY-MM-DD"T"HH24:MI:SS') AS name) x), 'weekly-baseline', 1)
ON CONFLICT DO NOTHING;
SQL

echo "==> Writing agent tokens for start-demo.sh..."
echo "export AGENT_TOKEN_1='$WEB01_TOKEN'" > /tmp/agent-tokens.env
echo "export AGENT_TOKEN_2='$DB01_TOKEN'" >> /tmp/agent-tokens.env
echo "export AGENT_TOKEN_3='$MEDIA_TOKEN'" >> /tmp/agent-tokens.env

echo "==> Demo data seeded successfully."
