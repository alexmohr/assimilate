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

# Triggers a repo sync, tolerating a 409 ("sync already in progress"). Repos
# with a sync_schedule are picked up immediately by the scheduler once
# configured (by design, so a never-synced repo starts syncing as soon as a
# schedule is set), so this explicit sync call can legitimately race with
# that scheduler-initiated sync. Either way the repo ends up syncing, which
# is all callers here actually need; wait_for_imports() below waits for
# whichever sync is in flight to finish.
sync_repo() {
    STATUS=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE_URL/api/repos/$1/sync" -H "$AUTH_HEADER")
    if [ "$STATUS" != "202" ] && [ "$STATUS" != "409" ]; then
        echo "sync request for repo $1 failed with status $STATUS" >&2
        exit 1
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
DELETE FROM backup_reports WHERE agent_id IN (SELECT id FROM agents WHERE hostname IN ('web-server-01','db-server-01','media-store-01','old-webserver','legacy-db-prod'));
DELETE FROM schedules WHERE id IN (SELECT st.schedule_id FROM schedule_targets st JOIN agents c ON c.id = st.agent_id WHERE c.hostname IN ('web-server-01','db-server-01','media-store-01'));
DELETE FROM ssh_tunnels WHERE agent_id IN (SELECT id FROM agents WHERE hostname IN ('web-server-01','db-server-01','media-store-01'));
DELETE FROM agent_hostname_patterns WHERE agent_id IN (SELECT id FROM agents WHERE hostname IN ('web-server-01','db-server-01','media-store-01'));
DELETE FROM agents WHERE hostname IN ('web-server-01','db-server-01','media-store-01','old-webserver','legacy-db-prod','unassigned-01','offline-due-01','disabled-only-01');
DELETE FROM repo_quotas WHERE repo_id IN (SELECT id FROM repos WHERE name IN ('server-daily','database-hourly','media-weekly'));
DELETE FROM server_quotas WHERE ssh_host = 'localhost';
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
INSERT INTO users (username, password_hash, must_change_password)
VALUES ('admin', '$2b$10$HvauZloS2N8QIfViDXmtp.rpWOawMeLdgWdBQQDHl3jD7Mhw7C3/e', false)
ON CONFLICT (username) DO NOTHING;
INSERT INTO user_roles (user_id, role_id)
SELECT u.id, r.id FROM users u, roles r WHERE u.username = 'admin' AND r.name = 'admin'
ON CONFLICT DO NOTHING;
SQL

echo "==> Logging in..."
login

echo "==> Setting timezone to Europe/Berlin..."
api PUT /api/system/settings '{"timezone":"Europe/Berlin","retention_days":7,"report_retention_days":365,"failed_report_retention_days":365,"system_event_retention_days":90}'

echo "==> Registering hosts for protected, unassigned, never-succeeded, and disabled-only coverage filters..."
WEB01_TOKEN=$(api POST "/api/agents" '{"hostname":"web-server-01","display_name":"Production Web Server"}' | jq -r '.token')
DB01_TOKEN=$(api POST "/api/agents" '{"hostname":"db-server-01","display_name":"Primary Database"}' | jq -r '.token')
MEDIA_TOKEN=$(api POST "/api/agents" '{"hostname":"media-store-01","display_name":"Media Storage NAS"}' | jq -r '.token')
api POST "/api/agents" '{"hostname":"unassigned-01","display_name":"Unassigned Demo Agent"}' > /dev/null
api POST "/api/agents" '{"hostname":"offline-due-01","display_name":"Offline Due Soon"}' > /dev/null
api POST "/api/agents" '{"hostname":"disabled-only-01","display_name":"Disabled Schedule Agent"}' > /dev/null

export AGENT_TOKEN_1="$WEB01_TOKEN"
export AGENT_TOKEN_2="$DB01_TOKEN"
export AGENT_TOKEN_3="$MEDIA_TOKEN"

echo "==> Setting an agent-level default file change pattern on db-server-01 (fallback for every schedule targeting this host)..."
api PUT "/api/agents/db-server-01" '{
    "display_name": "Primary Database",
    "default_file_change_patterns_raw": "*/var/lib/postgresql/*.tmp* ignore\n*checkpoint_wal* warn"
}' > /dev/null

echo "==> Registering repositories..."
REPO_DAILY_ID=$(api POST "/api/repos" "{
    \"name\": \"server-daily\",
    \"repo_path\": \"/backup/repos/server-daily\",
    \"ssh_user\": \"borg\",
    \"ssh_host\": \"localhost\",
    \"ssh_port\": 22,
    \"passphrase\": \"demo-passphrase-123\",
    \"compression\": \"lz4\"
}" | jq -r '.id')

REPO_HOURLY_ID=$(api POST "/api/repos" "{
    \"name\": \"database-hourly\",
    \"repo_path\": \"/backup/repos/database-hourly\",
    \"ssh_user\": \"borg\",
    \"ssh_host\": \"localhost\",
    \"ssh_port\": 22,
    \"passphrase\": \"demo-passphrase-123\",
    \"compression\": \"zstd,3\"
}" | jq -r '.id')

REPO_WEEKLY_ID=$(api POST "/api/repos" "{
    \"name\": \"media-weekly\",
    \"repo_path\": \"/backup/repos/media-weekly\",
    \"ssh_user\": \"borg\",
    \"ssh_host\": \"localhost\",
    \"ssh_port\": 22,
    \"passphrase\": \"demo-passphrase-123\",
    \"compression\": \"lz4\"
}" | jq -r '.id')

PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -v ON_ERROR_STOP=1 <<'SQL' > /dev/null
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM repos
        WHERE name IN ('server-daily', 'database-hourly', 'media-weekly')
          AND ssh_host_key IS NULL
    ) THEN
        RAISE EXCEPTION 'demo repositories must have pinned SSH host keys';
    END IF;
END
$$;
SQL

echo "==> Writing agent tokens for agent containers..."
mkdir -p /seeds
echo "AGENT_TOKEN_1='$WEB01_TOKEN'" > /seeds/tokens.env
echo "AGENT_TOKEN_2='$DB01_TOKEN'" >> /seeds/tokens.env
echo "AGENT_TOKEN_3='$MEDIA_TOKEN'" >> /seeds/tokens.env

echo "==> Signaling agent containers to start creating archives..."
touch /seeds/repos-ready

echo "==> Waiting for all agent/placeholder containers to finish creating archives..."
for HOST in web-server-01 db-server-01 media-store-01 old-webserver legacy-db-prod; do
    while [ ! -f "/seeds/done-$HOST" ]; do
        sleep 2
    done
    echo "  [$HOST] done."
done

# Configured only now, after every placeholder/agent container has finished
# writing its archives (including legacy-db-prod's unmatched archive into
# database-hourly) - not right after the repos are registered. sync_schedule
# is immediately "due" the moment it's set (last_synced_at is still NULL), so
# setting it earlier lets the server's own background scheduler race the
# placeholder containers: it can sync database-hourly before legacy-db-prod's
# archive exists, clear importing, and update last_synced_at - after which
# nothing re-syncs the repo until the next 4-hour cron boundary, permanently
# missing the archive the "unmatched-banner" E2E test depends on. The
# explicit sync_repo calls below tolerate a 409 from a concurrent sync
# without verifying it actually saw every archive, so they can't catch this
# on their own.
echo "==> Configuring per-repo sync schedules..."
api PUT "/api/repos/$REPO_HOURLY_ID" "{
    \"repo_path\": \"/backup/repos/database-hourly\",
    \"ssh_user\": \"borg\",
    \"ssh_host\": \"localhost\",
    \"ssh_port\": 22,
    \"compression\": \"zstd,3\",
    \"sync_schedule\": \"0 */4 * * *\"
}" > /dev/null

api PUT "/api/repos/$REPO_WEEKLY_ID" "{
    \"repo_path\": \"/backup/repos/media-weekly\",
    \"ssh_user\": \"borg\",
    \"ssh_host\": \"localhost\",
    \"ssh_port\": 22,
    \"compression\": \"lz4\",
    \"sync_schedule\": null
}" > /dev/null

# Blocks until no repo reports importing == true (sync runs in the background).
wait_for_imports() {
    for _attempt in $(seq 1 120); do
        still=$(curl -sf "$BASE_URL/api/repos" -H "$AUTH_HEADER" \
            | jq '[.[] | select(.importing == true)] | length' 2>/dev/null || echo 1)
        [ "$still" = "0" ] && break
        sleep 2
    done
}

# Blocks until background borg-info enrichment has populated sizes for all
# imported archives. Enrichment runs fire-and-forget after sync_existing_archives
# returns, so the importing flag clears before it completes. E2E tests must not
# start while borg info processes are still running and loading the server.
wait_for_enrichment() {
    for _attempt in $(seq 1 120); do
        pending=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc \
            "SELECT COUNT(*) FROM backup_reports WHERE original_size = 0 AND compressed_size = 0 AND deduplicated_size = 0 AND repo_id IN (SELECT id FROM repos WHERE name IN ('server-daily','database-hourly','media-weekly'))" 2>/dev/null || echo 1)
        [ "$pending" = "0" ] && break
        sleep 2
    done
}

echo "==> Syncing repos to import borg archives..."
sync_repo "$REPO_DAILY_ID"
sync_repo "$REPO_HOURLY_ID"
sync_repo "$REPO_WEEKLY_ID"

echo "==> Waiting for archive import to complete..."
wait_for_imports

echo "==> Waiting for background stat enrichment to complete..."
wait_for_enrichment

echo "==> Fetching agent IDs..."
WEB01_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM agents WHERE hostname='web-server-01'")
DB01_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM agents WHERE hostname='db-server-01'")
MEDIA_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM agents WHERE hostname='media-store-01'")
OFFLINE_DUE_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM agents WHERE hostname='offline-due-01'")
DISABLED_ONLY_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM agents WHERE hostname='disabled-only-01'")

echo "==> Creating schedules..."
api POST "/api/schedules" "{
    \"agent_ids\": [$WEB01_ID],
    \"repo_id\": $REPO_DAILY_ID,
    \"cron_expression\": \"0 2 * * *\",
    \"enabled\": true,
    \"keep_hourly\": 0,
    \"keep_daily\": 7,
    \"keep_weekly\": 4,
    \"keep_monthly\": 6,
    \"backup_sources\": [\"/var/www\", \"/etc/nginx\"],
    \"file_change_patterns_raw\": \"*/var/log/nginx/access.log* ignore\n*/var/www/cache* fatal\n*/etc/nginx/nginx.conf* warn\"
}" > /dev/null

api POST "/api/schedules" "{
    \"name\": \"Offline agent due soon\",
    \"agent_ids\": [$OFFLINE_DUE_ID],
    \"repo_id\": $REPO_DAILY_ID,
    \"cron_expression\": \"*/30 * * * *\",
    \"enabled\": true,
    \"keep_hourly\": 24,
    \"keep_daily\": 7,
    \"keep_weekly\": 4,
    \"keep_monthly\": 6,
    \"backup_sources\": [\"/etc\"]
}" > /dev/null

api POST "/api/schedules" "{
    \"name\": \"Disabled only coverage\",
    \"agent_ids\": [$DISABLED_ONLY_ID],
    \"repo_id\": $REPO_DAILY_ID,
    \"cron_expression\": \"0 1 * * *\",
    \"enabled\": false,
    \"keep_hourly\": 0,
    \"keep_daily\": 7,
    \"keep_weekly\": 4,
    \"keep_monthly\": 6,
    \"backup_sources\": [\"/srv\"]
}" > /dev/null

# next_run_at must stay within the dashboard's 2-hour "due soon" window (now..now+2h) at
# whatever wall-clock time Playwright actually visits the dashboard, not just at seed time -
# the e2e job's own image build/container startup can eat well over 30 minutes on a slow
# runner, which used to push next_run_at into the past before any test ever checked it,
# making the "host offline, due soon" dashboard finding disappear non-deterministically.
# 100 minutes leaves a comfortable margin against that startup delay while staying safely
# under the 2-hour ceiling.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
UPDATE schedules
SET last_run_at = NOW() - interval '45 minutes',
    next_run_at = NOW() + interval '100 minutes'
WHERE name = 'Offline agent due soon';

SQL

api POST "/api/schedules" "{
    \"agent_ids\": [$DB01_ID],
    \"repo_id\": $REPO_HOURLY_ID,
    \"cron_expression\": \"0 * * * *\",
    \"enabled\": true,
    \"keep_hourly\": 48,
    \"keep_daily\": 14,
    \"keep_weekly\": 8,
    \"keep_monthly\": 12,
    \"pre_backup_commands\": [\"echo '-- demo pg_dump $(date)' > /tmp/mydb.sql\"],
    \"backup_sources\": [\"/tmp/mydb.sql\", \"/var/lib/postgresql\"],
    \"rate_limit_kbps\": 5000
}" > /dev/null

api POST "/api/schedules" "{
    \"agent_ids\": [$MEDIA_ID],
    \"repo_id\": $REPO_WEEKLY_ID,
    \"cron_expression\": \"0 3 * * 0\",
    \"enabled\": true,
    \"keep_hourly\": 0,
    \"keep_daily\": 0,
    \"keep_weekly\": 4,
    \"keep_monthly\": 12,
    \"keep_yearly\": 2,
    \"backup_sources\": [\"/mnt/media/photos\", \"/mnt/media/videos\"]
}" > /dev/null

api POST "/api/schedules" "{
    \"agent_ids\": [$WEB01_ID, $DB01_ID, $MEDIA_ID],
    \"repo_id\": $REPO_DAILY_ID,
    \"cron_expression\": \"0 4 * * *\",
    \"enabled\": true,
    \"execution_mode\": \"sequential\",
    \"on_failure\": \"stop\",
    \"keep_hourly\": 24,
    \"keep_daily\": 7,
    \"keep_weekly\": 4,
    \"keep_monthly\": 6,
    \"backup_sources\": [\"/etc\"],
    \"backup_sources_per_agent\": [
        {\"agent_id\": $WEB01_ID, \"paths\": [\"/var/www\", \"/etc/nginx\", \"/var/log/nginx\"]},
        {\"agent_id\": $DB01_ID, \"paths\": [\"/var/lib/postgresql\", \"/etc/postgresql\"]},
        {\"agent_id\": $MEDIA_ID, \"paths\": [\"/mnt/media/photos\", \"/mnt/media/videos\"]}
    ],
    \"exclude_patterns_per_agent\": [
        {\"agent_id\": $WEB01_ID, \"raw_text\": \"*.log\"},
        {\"agent_id\": $DB01_ID, \"raw_text\": \"*.tmp\"}
    ],
    \"file_change_patterns_raw\": \"*/var/log/nginx/access.log* ignore\n*/var/www/cache* fatal\n*/etc/nginx/nginx.conf* warn\",
    \"file_change_patterns_per_agent\": [
        {\"agent_id\": $WEB01_ID, \"raw_text\": \"*/var/log/nginx/error.log* ignore\"}
    ]
}" > /dev/null

PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
UPDATE backup_reports br
SET schedule_id = s.id
FROM schedules s
JOIN schedule_targets st ON st.schedule_id = s.id
WHERE br.schedule_id IS NULL
  AND br.repo_id = s.repo_id
  AND br.agent_id = st.agent_id
  AND s.enabled = true
  AND s.name <> 'Offline agent due soon';
SQL

echo "==> Adding global excludes..."
# /api/excludes stores a single raw_text blob (one pattern per line) - it is not
# a per-pattern collection endpoint.
EXCLUDES_RAW_TEXT="pp:__pycache__
pp:.cache
pp:node_modules
*.pyc
*.swp
*~
/proc
/sys
/tmp"
api PUT "/api/excludes" "$(jq -n --arg raw_text "$EXCLUDES_RAW_TEXT" '{raw_text: $raw_text}')" > /dev/null

echo "==> Creating tags..."
api POST "/api/tags" '{"name":"production","color":"#ef4444","scope":"agent"}' > /dev/null 2>&1 || true
api POST "/api/tags" '{"name":"staging","color":"#f59e0b","scope":"agent"}' > /dev/null 2>&1 || true
api POST "/api/tags" '{"name":"critical","color":"#dc2626","scope":"repo"}' > /dev/null 2>&1 || true
api POST "/api/tags" '{"name":"archival","color":"#6366f1","scope":"repo"}' > /dev/null 2>&1 || true

echo "==> Associating repo tags (for config-export coverage)..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO repo_tags (repo_id, tag_id)
SELECT $REPO_DAILY_ID, t.id FROM tags t WHERE t.name = 'critical' AND t.scope = 'repo'
ON CONFLICT DO NOTHING;
INSERT INTO repo_tags (repo_id, tag_id)
SELECT $REPO_WEEKLY_ID, t.id FROM tags t WHERE t.name = 'archival' AND t.scope = 'repo'
ON CONFLICT DO NOTHING;
SQL

echo "==> Creating additional users and roles..."
# Passwords match the usernames (bcrypt cost 10, pre-computed), the same convention
# used for the admin account above, so e2e RBAC tests can log in as these roles.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<'SQL'
INSERT INTO users (username, password_hash) VALUES
    ('operator1', '$2b$10$bO6/.9GSDqqTPFqe1CiOGOf2UZt3rxK71x7CfBXlFotSLhT0aUoZ2'),
    ('viewer1', '$2b$10$Ex5wHmqtI7IFdor4vJdXo.6YvqGErhf3PtiKGKCDORiArpZwyg3Ze')
ON CONFLICT (username) DO UPDATE SET password_hash = EXCLUDED.password_hash;
INSERT INTO user_roles (user_id, role_id)
SELECT u.id, r.id FROM users u, roles r WHERE u.username = 'operator1' AND r.name = 'operator'
ON CONFLICT DO NOTHING;
INSERT INTO user_roles (user_id, role_id)
SELECT u.id, r.id FROM users u, roles r WHERE u.username = 'viewer1' AND r.name = 'viewer'
ON CONFLICT DO NOTHING;
SQL

echo "==> Creating groups..."
api POST "/api/groups" '{"name":"backend-team","description":"Backend infrastructure engineers"}' > /dev/null 2>&1 || true
api POST "/api/groups" '{"name":"data-team","description":"Database and analytics team"}' > /dev/null 2>&1 || true

echo "==> Configuring hostname pattern matching..."
# Demonstrates pattern-based agent matching: archives from any 'web-server-*'
# agent resolve to web-server-01.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO agent_hostname_patterns (agent_id, pattern)
SELECT id, 'web-server-*' FROM agents WHERE hostname='web-server-01'
ON CONFLICT DO NOTHING;
SQL

echo "==> Setting up repo quotas..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO repo_quotas (repo_id, warn_bytes, critical_bytes, warn_action, critical_action, enabled) VALUES
    ($REPO_DAILY_ID, 10737418240, 16106127360, 'notify_only', 'block_backups', true),
    ($REPO_HOURLY_ID, 5368709120, 8589934592, 'notify_only', 'disable_schedule', true),
    ($REPO_WEEKLY_ID, 1, 1099511627776, 'notify_only', 'notify_only', true)
ON CONFLICT (repo_id) DO NOTHING;
SQL

echo "==> Setting up server quota (shared localhost host)..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO server_quotas (ssh_host, warn_bytes, critical_bytes, warn_action, critical_action, enabled) VALUES
    ('localhost', 21474836480, 32212254720, 'notify_only', 'block_backups', true)
ON CONFLICT (ssh_host) DO NOTHING;
SQL

echo "==> Adding system events..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO system_events (created_at, event_type, hostname, message) VALUES
    (NOW() - interval '5 minutes', 'agent_connected', 'web-server-01', 'Agent connected (version 0.1.0)'),
    (NOW() - interval '4 minutes', 'agent_connected', 'db-server-01', 'Agent connected (version 0.1.0)'),
    (NOW() - interval '3 minutes', 'agent_connected', 'media-store-01', 'Agent connected (version 0.1.0)'),
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
    (1, 'admin', 'agent.create', 'agent', $WEB01_ID, '{"hostname":"web-server-01"}', '192.168.1.10', NOW() - interval '28 days'),
    (1, 'admin', 'agent.create', 'agent', $DB01_ID, '{"hostname":"db-server-01"}', '192.168.1.10', NOW() - interval '28 days'),
    (1, 'admin', 'agent.create', 'agent', $MEDIA_ID, '{"hostname":"media-store-01"}', '192.168.1.10', NOW() - interval '27 days'),
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

echo "==> Adding SSH tunnel entry for loopback agent communication..."
api POST "/api/tunnels" "{\"agent_id\":$MEDIA_ID,\"ssh_host\":\"127.0.0.1\",\"ssh_user\":\"borg\",\"ssh_port\":22,\"tunnel_port\":18080,\"enabled\":true}" > /dev/null

echo "==> Adding archive tags..."
# Tag real imported archives (by joining backup_reports -> archives) rather than
# guessing names -- the most recent and 3rd-most-recent web-server-01 archives.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO archive_tags (archive_id, tag, created_by)
SELECT a.id, 'pre-upgrade', 1
FROM archives a
JOIN backup_reports br ON a.repo_id = br.repo_id AND a.name = br.archive_name
WHERE br.repo_id = $REPO_DAILY_ID AND br.archive_name LIKE 'web-server-01-backup-%'
ORDER BY br.started_at DESC LIMIT 1
ON CONFLICT DO NOTHING;

INSERT INTO archive_tags (archive_id, tag, created_by)
SELECT a.id, 'weekly-baseline', 1
FROM archives a
JOIN backup_reports br ON a.repo_id = br.repo_id AND a.name = br.archive_name
WHERE br.repo_id = $REPO_DAILY_ID AND br.archive_name LIKE 'web-server-01-backup-%'
ORDER BY br.started_at DESC OFFSET 2 LIMIT 1
ON CONFLICT DO NOTHING;
SQL

echo "==> Adding warnings to the most recent web-server-01 backup report..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -v ON_ERROR_STOP=1 <<'SQL' > /dev/null
UPDATE backup_reports
SET warnings = ARRAY[
        'file changed while we backed it up: /var/www/config.php',
        'slow read on /var/log/nginx/access.log'
    ],
    status = 'warning'
WHERE id = (
    SELECT id FROM backup_reports
    WHERE agent_id = (SELECT id FROM agents WHERE hostname = 'web-server-01')
      AND archive_name LIKE 'web-server-01-backup-%'
    ORDER BY started_at DESC
    LIMIT 1
);
SQL

echo "==> Updating database storage statistics..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -c 'ANALYZE;' > /dev/null

echo "==> Verifying config export/import round-trip (repos, tags, quotas)..."
EXPORT_JSON=$(api GET /api/config/export)
echo "$EXPORT_JSON" | jq -e '.repos | length > 0' > /dev/null || {
    echo "ERROR: config export should include at least one repo" >&2
    exit 1
}
IMPORT_RESULT=$(api POST /api/config/import "$EXPORT_JSON")
echo "$IMPORT_RESULT" | jq -e '.repos_updated > 0' > /dev/null && echo "  config import updated existing repos (expected)." || true

echo "==> Demo data seeded successfully."
