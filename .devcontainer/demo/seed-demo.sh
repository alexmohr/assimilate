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
for REPO_NAME in server-daily database-hourly media-weekly stale-report-repo; do
    REPO_DIR="/backup/repos/$REPO_NAME"
    if [ ! -d "$REPO_DIR" ]; then
        su -c "BORG_PASSPHRASE=demo-passphrase-123 borg init --encryption=repokey-blake2 $REPO_DIR" borg
    fi
done

echo "==> Cleaning up existing demo data (idempotent re-run)..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<'SQL' > /dev/null 2>&1
DELETE FROM backup_reports WHERE agent_id IN (SELECT id FROM agents WHERE hostname IN ('web-server-01','db-server-01','media-store-01','old-webserver','legacy-db-prod'));
DELETE FROM schedules WHERE id IN (SELECT st.schedule_id FROM schedule_targets st JOIN agents c ON c.id = st.agent_id WHERE c.hostname IN ('web-server-01','db-server-01','media-store-01'));
DELETE FROM schedules WHERE name = 'Stale nightly report';
DELETE FROM schedules WHERE name = 'Auto-disabled demo';
DELETE FROM schedules WHERE name = 'Missed backups warning demo';
DELETE FROM ssh_tunnels WHERE agent_id IN (SELECT id FROM agents WHERE hostname IN ('web-server-01','db-server-01','media-store-01'));
DELETE FROM agent_hostname_patterns WHERE agent_id IN (SELECT id FROM agents WHERE hostname IN ('web-server-01','db-server-01','media-store-01'));
DELETE FROM agents WHERE hostname IN ('web-server-01','db-server-01','media-store-01','old-webserver','legacy-db-prod','unassigned-01','offline-due-01','disabled-only-01','stale-report-01','auto-disabled-01','edge-proxy');
DELETE FROM repo_quotas WHERE repo_id IN (SELECT id FROM repos WHERE name IN ('server-daily','database-hourly','media-weekly','stale-report-repo'));
DELETE FROM server_quotas WHERE ssh_host = 'localhost';
DELETE FROM archive_tags WHERE repo_id IN (SELECT id FROM repos WHERE name IN ('server-daily','database-hourly','media-weekly','stale-report-repo'));
DELETE FROM notification_rules;
DELETE FROM notification_channels;
DELETE FROM repos WHERE name IN ('server-daily','database-hourly','media-weekly','stale-report-repo');
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

echo "==> Setting timezone to Europe/Berlin and configuring session idle timeout..."
api PUT /api/system/settings '{"timezone":"Europe/Berlin","retention_days":7,"report_retention_days":365,"failed_report_retention_days":365,"system_event_retention_days":90,"notification_delivery_retention_days":30,"session_idle_timeout_minutes":480}'

echo "==> Registering hosts for protected, unassigned, never-succeeded, and disabled-only coverage filters..."
WEB01_TOKEN=$(api POST "/api/agents" '{"hostname":"web-server-01","display_name":"Production Web Server"}' | jq -r '.token')
DB01_TOKEN=$(api POST "/api/agents" '{"hostname":"db-server-01","display_name":"Primary Database"}' | jq -r '.token')
MEDIA_TOKEN=$(api POST "/api/agents" '{"hostname":"media-store-01","display_name":"Media Storage NAS"}' | jq -r '.token')
api POST "/api/agents" '{"hostname":"unassigned-01","display_name":"Unassigned Demo Agent"}' > /dev/null
api POST "/api/agents" '{"hostname":"offline-due-01","display_name":"Offline Due Soon"}' > /dev/null
api POST "/api/agents" '{"hostname":"disabled-only-01","display_name":"Disabled Schedule Agent"}' > /dev/null
api POST "/api/agents" '{"hostname":"stale-report-01","display_name":"Stale Report Demo"}' > /dev/null
api POST "/api/agents" '{"hostname":"auto-disabled-01","display_name":"Auto-Disabled Demo"}' > /dev/null

echo "==> Registering two hosts sharing a hostname across different domains..."
api POST "/api/agents" '{"hostname":"edge-proxy","display_name":"Edge Proxy (DC1)","domain":"dc1.example.com"}' > /dev/null
api POST "/api/agents" '{"hostname":"edge-proxy","display_name":"Edge Proxy (DC2)","domain":"dc2.example.com"}' > /dev/null

export AGENT_TOKEN_1="$WEB01_TOKEN"
export AGENT_TOKEN_2="$DB01_TOKEN"
export AGENT_TOKEN_3="$MEDIA_TOKEN"

echo "==> Setting an agent-level default file change pattern on db-server-01 (fallback for every schedule targeting this host)..."
api PUT "/api/agents/db-server-01" '{
    "display_name": "Primary Database",
    "default_file_change_patterns_raw": "*/var/lib/postgresql/*.tmp* ignore\n*checkpoint_wal* warn"
}' > /dev/null

# Agent-level hook commands, so the Backup defaults pane shows the read-only
# script block: a multi-line pre-backup command (which an inline <code> would
# collapse onto one line) carrying its own generous timeout, next to a
# post-backup one that inherits whatever the schedule sets.
echo "==> Setting agent-level default hook commands on media-store-01 (one with its own timeout)..."
api PUT "/api/agents/media-store-01" '{
    "display_name": "Media Store",
    "default_pre_backup_commands": [
        {
            "command": "# make sure the media share is really mounted\nif ! mountpoint -q /mnt/media; then\n    mount /mnt/media || exit 1\nfi\nfind /mnt/media -name \"*.part\" -delete",
            "timeout_seconds": 7200
        }
    ],
    "default_post_backup_commands": [
        {"command": "umount /mnt/media", "timeout_seconds": null}
    ]
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

# Dedicated repo for the stale-report-01 demo below, never shared with a
# schedule that runs real backups and never synced. list_archive_names_for_repo
# treats every backup_reports.archive_name for a repo as a "known" archive, and
# a full/scheduled sync (sync_existing_archives, SyncMode::Existing) deletes any
# backup_reports row whose archive_name isn't found in a real `borg list` of
# that repo (see delete_archive_records_by_names in db/mod.rs). The backdated
# report seeded below has no matching real archive on disk, so if it shared a
# repo that ever gets synced (like server-daily, synced for web-server-01's
# real backups), it gets silently deleted by that reconciliation within
# minutes - this is what caused the Retry-button e2e test to fail against the
# live demo despite passing every local check.
STALE_REPORT_REPO_ID=$(api POST "/api/repos" "{
    \"name\": \"stale-report-repo\",
    \"repo_path\": \"/backup/repos/stale-report-repo\",
    \"ssh_user\": \"borg\",
    \"ssh_host\": \"localhost\",
    \"ssh_port\": 22,
    \"passphrase\": \"demo-passphrase-123\",
    \"compression\": \"lz4\"
}" | jq -r '.id')
api PUT "/api/repos/$STALE_REPORT_REPO_ID" "{
    \"repo_path\": \"/backup/repos/stale-report-repo\",
    \"ssh_user\": \"borg\",
    \"ssh_host\": \"localhost\",
    \"ssh_port\": 22,
    \"sync_schedule\": null
}" > /dev/null

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
# Must poll /api/repos/stats, not /api/repos: the latter returns the plain
# RepoResponse DTO, which has no `importing` field at all, so `.importing`
# is always null there and this loop would break on its very first
# iteration regardless of whether a sync is actually still running -
# exactly the bug that made every downstream step (schedule_id backfill,
# archive tagging, the dashboard's per-schedule history) silently race
# against archives that hadn't been imported yet.
wait_for_imports() {
    for _attempt in $(seq 1 120); do
        still=$(curl -sf "$BASE_URL/api/repos/stats" -H "$AUTH_HEADER" \
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
STALE_REPORT_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM agents WHERE hostname='stale-report-01'")
AUTO_DISABLED_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc "SELECT id FROM agents WHERE hostname='auto-disabled-01'")

# media-store-01 is the "not always on" host in this demo, so it's also the
# one with wake/shutdown configured - giving the agent and repository Power
# settings panes, and the run timeline they feed, real data to show. Its
# agent runs as a persistent service started by start-agent.sh, not deployed
# by the server over SSH, so last_ssh_user is never set by the normal deploy
# flow - set it directly here, standing in for "an admin has already
# verified SSH access to this host", which shutdown_after_backup requires.
echo "==> Configuring power management..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -c \
    "UPDATE agents SET last_ssh_user = 'borg' WHERE hostname = 'media-store-01'" > /dev/null
api PUT "/api/agents/media-store-01/power" '{
    "wake": {
        "wake_enabled": true,
        "wake_mac_address": "3C:97:0E:2B:9A:44",
        "wake_broadcast_address": "192.168.1.255",
        "wake_timeout_seconds": 180,
        "shutdown_after_backup": true
    },
    "start_agent_enabled": false,
    "stop_agent_after_backup": false,
    "ssh_host": "media-store-01",
    "ssh_port": 22,
    "agent_service_name": "assimilate-agent"
}' > /dev/null
api PUT "/api/repos/$REPO_WEEKLY_ID/power" '{
    "wake_enabled": true,
    "wake_mac_address": "9C:B6:D0:1A:44:7F",
    "wake_broadcast_address": "192.168.1.255",
    "wake_timeout_seconds": 240,
    "shutdown_after_backup": true
}' > /dev/null

echo "==> Creating schedules..."
WEB01_DAILY_SCHEDULE_ID=$(api POST "/api/schedules" "{
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
}" | jq -r '.id')

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

api POST "/api/schedules" "{
    \"name\": \"Auto-disabled demo\",
    \"agent_ids\": [$AUTO_DISABLED_ID],
    \"repo_id\": $REPO_DAILY_ID,
    \"cron_expression\": \"0 6 * * *\",
    \"enabled\": true,
    \"keep_hourly\": 0,
    \"keep_daily\": 7,
    \"keep_weekly\": 4,
    \"keep_monthly\": 6,
    \"backup_sources\": [\"/srv/auto-disabled-demo\"]
}" > /dev/null

# Demonstrates the "Auto-disabled" status pill and its System Events entry
# (see docs/agents.md) by directly reproducing the bookkeeping the scheduler
# itself would write after 3 consecutive failures to reach this agent, rather
# than actually waiting out real backoff ticks against an agent that never
# connects.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -v ON_ERROR_STOP=1 <<SQL
UPDATE schedules
SET enabled = false,
    auto_disabled_agent_unreachable = true,
    auto_disabled_by_agent_id = $AUTO_DISABLED_ID,
    consecutive_failures = 3,
    failure_streak_pure_connectivity = true
WHERE name = 'Auto-disabled demo';

INSERT INTO system_events (event_type, hostname, message)
SELECT 'schedule_auto_disabled', 'auto-disabled-01',
       'Schedule ''Auto-disabled demo'' auto-disabled after 3 consecutive failures: agent ''auto-disabled-01'' stayed unreachable'
WHERE EXISTS (SELECT 1 FROM schedules WHERE name = 'Auto-disabled demo');
SQL

api POST "/api/schedules" "{
    \"name\": \"Missed backups warning demo\",
    \"agent_ids\": [$AUTO_DISABLED_ID],
    \"repo_id\": $REPO_DAILY_ID,
    \"cron_expression\": \"0 7 * * *\",
    \"enabled\": true,
    \"keep_hourly\": 0,
    \"keep_daily\": 7,
    \"keep_weekly\": 4,
    \"keep_monthly\": 6,
    \"missed_backup_threshold\": 3,
    \"backup_sources\": [\"/srv/missed-backups-demo\"]
}" > /dev/null

# Demonstrates the "N/threshold missed" warning chip (see
# docs/scheduling.md#missed-backup-threshold) by directly writing a below-threshold
# consecutive_failures count, the same way the fully auto-disabled schedule above
# simulates 3 consecutive failures. One miss short of this schedule's own threshold
# of 3, so it stays enabled with a warning instead of Auto-disabled.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -v ON_ERROR_STOP=1 <<SQL
UPDATE schedules
SET consecutive_failures = 2,
    failure_streak_pure_connectivity = true
WHERE name = 'Missed backups warning demo';
SQL

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

# Second run into the *same* repository as 'Offline agent due soon', five
# minutes after it, so the Schedules page's 24-hour rail always has a
# collision to warn about and to expand (see docs/scheduling.md). It has to be
# the same repository, not merely the same storage host: every demo repo lives
# on localhost, so a host-keyed warning would fire for every pair of runs on
# the page and say nothing.
WEB01_COLLIDING_SCHEDULE_ID=$(api POST "/api/schedules" "{
    \"name\": \"Colliding daily window\",
    \"agent_ids\": [$WEB01_ID],
    \"repo_id\": $REPO_DAILY_ID,
    \"cron_expression\": \"10 2 * * *\",
    \"enabled\": true,
    \"keep_hourly\": 0,
    \"keep_daily\": 7,
    \"keep_weekly\": 4,
    \"keep_monthly\": 6,
    \"backup_sources\": [\"/etc\"]
}" | jq -r '.id')

PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
UPDATE schedules
SET last_run_at = NULL,
    next_run_at = NOW() + interval '105 minutes'
WHERE name = 'Colliding daily window';

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
    \"pre_backup_commands\": [
        {\"command\": \"echo '-- demo pg_dump $(date)' > /tmp/mydb.sql\", \"timeout_seconds\": 1800},
        {\"command\": \"df -hP /var/lib/postgresql | tail -n1 | awk '{print \$5}' > /tmp/db-disk-usage.txt\\necho \\\"disk usage recorded: \$(cat /tmp/db-disk-usage.txt)\\\"\", \"timeout_seconds\": null}
    ],
    \"post_backup_commands\": [{\"command\": \"rm -f /tmp/mydb.sql /tmp/db-disk-usage.txt\", \"timeout_seconds\": null}],
    \"backup_sources\": [\"/tmp/mydb.sql\", \"/var/lib/postgresql\"],
    \"rate_limit_kbps\": 5000,
    \"hook_timeout_seconds\": 120
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

# The multi-host schedule. db-server-01 and media-store-01 each write two
# archives into server-daily (see start-agent.sh), so this schedule's Backups
# tab has archives from more than one host - which is what makes the archive
# selector's host grouping, and its per-host totals, visible there.
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

api POST "/api/schedules" "{
    \"name\": \"Stale nightly report\",
    \"agent_ids\": [$STALE_REPORT_ID],
    \"repo_id\": $STALE_REPORT_REPO_ID,
    \"cron_expression\": \"0 5 * * *\",
    \"enabled\": true,
    \"keep_hourly\": 0,
    \"keep_daily\": 7,
    \"keep_weekly\": 4,
    \"keep_monthly\": 6,
    \"backup_sources\": [\"/opt/app\"]
}" > /dev/null

# Demonstrates the Schedules page's "N host(s) overdue" expand toggle: the
# schedule's own last_run_at/next_run_at look on track (last dispatch a
# couple hours ago, next one tonight), but this target host's own most
# recent backup report is old enough that it's overdue for a daily cron
# (see is_overdue() in crates/server/src/api/stats.rs) - the exact
# "looks fine but the badge says Overdue" scenario the toggle exists to
# explain. No error_message on the report, since this host's problem is
# staleness, not a failure.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -v ON_ERROR_STOP=1 <<SQL
INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, status, archive_name)
SELECT $STALE_REPORT_ID, $STALE_REPORT_REPO_ID, s.id,
       NOW() - interval '4 days' - interval '5 minutes', NOW() - interval '4 days',
       'success', 'stale-report-01-backup-old'
FROM schedules s WHERE s.name = 'Stale nightly report';

UPDATE schedules
SET last_run_at = NOW() - interval '2 hours',
    next_run_at = NOW() + interval '10 hours'
WHERE name = 'Stale nightly report';
SQL

# Fail loudly here rather than leaving the Retry-button e2e test to fail with
# a confusing "Overdue badge never appeared" 20+ minutes later - this makes a
# silently-empty INSERT (e.g. no schedule matched the SELECT) diagnosable
# from the seed step itself instead of guessed at from a downstream test.
STALE_REPORT_COUNT=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc \
    "SELECT COUNT(*) FROM backup_reports WHERE agent_id = $STALE_REPORT_ID AND archive_name = 'stale-report-01-backup-old'")
if [ "$STALE_REPORT_COUNT" != "1" ]; then
    echo "expected exactly 1 backdated backup_reports row for stale-report-01, found $STALE_REPORT_COUNT" >&2
    exit 1
fi

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
    ('viewer1', '$2b$10$Ex5wHmqtI7IFdor4vJdXo.6YvqGErhf3PtiKGKCDORiArpZwyg3Ze'),
    ('totpuser', '$2b$10$92LXqE0n28dyZnMu3ZALt.EsjPxgzLcjcOL4Oapg.mGLah7y65bW2')
ON CONFLICT (username) DO UPDATE SET password_hash = EXCLUDED.password_hash;
INSERT INTO user_roles (user_id, role_id)
SELECT u.id, r.id FROM users u, roles r WHERE u.username = 'operator1' AND r.name = 'operator'
ON CONFLICT DO NOTHING;
INSERT INTO user_roles (user_id, role_id)
SELECT u.id, r.id FROM users u, roles r WHERE u.username = 'viewer1' AND r.name = 'viewer'
ON CONFLICT DO NOTHING;
INSERT INTO user_roles (user_id, role_id)
SELECT u.id, r.id FROM users u, roles r WHERE u.username = 'totpuser' AND r.name = 'viewer'
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
    (NOW() - interval '5 minutes', 'repo_sync', 'web-server-01', 'Repository sync completed'),
    (NOW() - interval '4 minutes', 'repo_sync', 'db-server-01', 'Repository sync completed'),
    (NOW() - interval '3 minutes', 'repo_sync', 'media-store-01', 'Repository sync completed'),
    (NOW() - interval '2 days', 'repo_sync_slow', 'media-store-01', 'Repository sync took longer than the warning threshold'),
    (NOW() - interval '7 days', 'repo_sync_failed', 'web-server-01', 'Repository sync failed: repository lock could not be acquired'),
    (NOW() - interval '9 days', 'repo_sync_failed', 'db-server-01', 'Repository sync failed: connection refused'),
    (NOW() - interval '1 day', 'auth_failed', 'web-server-01', 'Agent authentication failed: invalid token');
SQL

echo "==> Acknowledging the older failed sync, so both system-event states exist..."
# One acknowledged and one still-open sync failure: the acknowledged one is
# hidden until the Activity Log's Acknowledged filter asks for it, which is
# exactly what that filter's screenshot needs to show.
DB01_SYNC_EVENT_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc \
    "SELECT id FROM system_events
     WHERE event_type = 'repo_sync_failed' AND hostname = 'db-server-01'
     ORDER BY created_at DESC LIMIT 1")
if [ -z "$DB01_SYNC_EVENT_ID" ]; then
    echo "expected a db-server-01 repo_sync_failed event to acknowledge, found none" >&2
    exit 1
fi
api POST "/api/stats/system-events/$DB01_SYNC_EVENT_ID/acknowledge" > /dev/null

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
     (VALUES ('backup_failed'), ('backup_warning'), ('agent_disconnected'), ('schedule_auto_disabled'))
         AS e(event_type)
WHERE c.name = 'Ops Webhook';

INSERT INTO notification_rules (channel_id, event_type, enabled)
SELECT c.id, e.event_type, true
FROM notification_channels c,
     (VALUES ('backup_failed'), ('backup_success'), ('agent_connected'), ('agent_disconnected')) AS e(event_type)
WHERE c.name = 'Admin Email';
SQL

echo "==> Adding notification delivery history..."
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg <<SQL
INSERT INTO notification_deliveries (channel_id, event_type, payload, status, error_message, attempted_at)
SELECT c.id, 'backup_failed',
    '{"event_type":"backup_failed","hostname":"web-server-01","repo_name":"server-daily","status":"failed","error_message":"Repository lock could not be acquired","timestamp":"2026-01-15T03:00:12Z"}',
    'failed',
    'webhook delivery failed: could not resolve host: hooks.example.com',
    NOW() - interval '7 days'
FROM notification_channels c WHERE c.name = 'Ops Webhook';

INSERT INTO notification_deliveries (channel_id, event_type, payload, status, error_message, attempted_at)
SELECT c.id, 'backup_warning',
    '{"event_type":"backup_warning","hostname":"web-server-01","repo_name":"server-daily","status":"warning","timestamp":"2026-01-14T01:00:05Z"}',
    'sent',
    NULL,
    NOW() - interval '1 day'
FROM notification_channels c WHERE c.name = 'Ops Webhook';

INSERT INTO notification_deliveries (channel_id, event_type, payload, status, error_message, attempted_at)
SELECT c.id, 'schedule_auto_disabled',
    '{"event_type":"schedule_auto_disabled","hostname":"auto-disabled-01","repo_name":"server-daily","schedule_name":"Auto-disabled demo","status":"auto_disabled","error_message":"agent ''auto-disabled-01'' stayed unreachable","timestamp":"2026-01-13T06:00:00Z"}',
    'sent',
    NULL,
    NOW() - interval '2 days'
FROM notification_channels c WHERE c.name = 'Ops Webhook';
SQL

echo "==> Adding SSH tunnel entry for loopback agent communication..."
api POST "/api/tunnels" "{\"agent_id\":$MEDIA_ID,\"ssh_host\":\"127.0.0.1\",\"ssh_user\":\"borg\",\"ssh_port\":22,\"tunnel_port\":18080,\"enabled\":true}" > /dev/null

echo "==> Warming the archive content index..."
# Browsing an archive builds its content index in the background (one compressed
# blob per directory in archive_dirs). Doing it here means the archive browser
# has contents to show as soon as the demo comes up, instead of showing an
# "indexing" spinner on the first screenshot, and it exercises the packed
# storage layout end to end.
NEWEST_WEB01_ARCHIVE=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc \
    "SELECT br.archive_name FROM backup_reports br \
     WHERE br.repo_id = $REPO_DAILY_ID AND br.archive_name LIKE 'web-server-01-backup-%' \
     ORDER BY br.started_at DESC LIMIT 1")
api GET "/api/repos/$REPO_DAILY_ID/archives/$NEWEST_WEB01_ARCHIVE/contents" > /dev/null

# Wait on the indexing job's own status rather than on archive_dirs having rows.
# replace_archive_dirs deletes the archive's rows and re-inserts them in batches
# without a wrapping transaction, so a row count can be non-zero while the index
# is still half-written; the job only reaches 'done' once indexing has finished,
# which is the same signal the API uses before serving the stored index.
INDEX_WAIT=0
INDEX_STATUS=""
while [ "$INDEX_WAIT" -lt 60 ]; do
    INDEX_STATUS=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc \
        "SELECT j.status FROM archive_index_jobs j JOIN archives a ON a.id = j.archive_id \
         WHERE a.repo_id = $REPO_DAILY_ID AND a.name = '$NEWEST_WEB01_ARCHIVE'")
    case "$INDEX_STATUS" in
        done) break ;;
        failed)
            echo "archive content indexing for '$NEWEST_WEB01_ARCHIVE' failed" >&2
            exit 1
            ;;
        *) ;;
    esac
    sleep 1
    INDEX_WAIT=$((INDEX_WAIT + 1))
done
if [ "$INDEX_STATUS" != "done" ]; then
    echo "archive content index for '$NEWEST_WEB01_ARCHIVE' did not finish indexing" \
        "(last status: ${INDEX_STATUS:-none})" >&2
    exit 1
fi

# The job is done, so the blobs are fully written: check the packed layout really
# was populated and the demo browser has something to show.
INDEXED_DIRS=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc \
    "SELECT COUNT(*) FROM archive_dirs d JOIN archives a ON a.id = d.archive_id \
     WHERE a.repo_id = $REPO_DAILY_ID AND a.name = '$NEWEST_WEB01_ARCHIVE'")
if [ "$INDEXED_DIRS" -eq 0 ]; then
    echo "archive content index for '$NEWEST_WEB01_ARCHIVE' never populated archive_dirs" >&2
    exit 1
fi

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
# Also the demo's coverage of the recent-backups preview's "View warnings"
# jump (see docs/agents.md, docs/scheduling.md): this run is web-server-01's
# newest, so the button is on the first row of both the host's and the
# schedule's Overview preview. Keep it the newest if this block is edited.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -v ON_ERROR_STOP=1 <<'SQL' > /dev/null
-- The first warning is deliberately long-winded, since borg emits
-- multi-sentence diagnostics: the activity log shows it in full, while a
-- dashboard Needs Attention finding built from it gets capped and clamped
-- rather than growing a row as tall as the message happens to be.
UPDATE backup_reports
SET warnings = ARRAY[
        'file changed while we backed it up: /var/www/config.php - the size or inode changed between stat() and read(), so the archived copy may not match the file on disk; borg archived the version it read first and continued with the remaining backup sources',
        'slow read on /var/log/nginx/access.log'
    ],
    -- The agent populates error_message for warning-only runs too (the
    -- backup_warning notification path reads it); the UI hides the
    -- redundant Error box for warning-status reports instead of the agent
    -- dropping the message, so this mirrors real agent behavior.
    error_message = 'file changed while we backed it up: /var/www/config.php - the size or inode changed between stat() and read(), so the archived copy may not match the file on disk; borg archived the version it read first and continued with the remaining backup sources; slow read on /var/log/nginx/access.log',
    status = 'warning'
WHERE id = (
    SELECT id FROM backup_reports
    WHERE agent_id = (SELECT id FROM agents WHERE hostname = 'web-server-01')
      AND archive_name LIKE 'web-server-01-backup-%'
    ORDER BY started_at DESC
    LIMIT 1
);
SQL

echo "==> Acknowledging that warning, so the Activity Log demonstrates both states..."
# The remaining seeded failures/warnings (db-server-01's outage below, the
# hourly failure, the failed-report-cleanup targets) stay unacknowledged, so
# the activity screenshot shows what still needs attention, while switching
# the Acknowledged filter to Shown reveals this reviewed one alongside them.
WEB01_WARNING_REPORT_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc \
    "SELECT id FROM backup_reports
     WHERE agent_id = (SELECT id FROM agents WHERE hostname = 'web-server-01')
       AND archive_name LIKE 'web-server-01-backup-%'
     ORDER BY started_at DESC LIMIT 1")
if [ -z "$WEB01_WARNING_REPORT_ID" ]; then
    echo "expected a web-server-01 backup report to acknowledge, found none" >&2
    exit 1
fi
api POST "/api/stats/activity/$WEB01_WARNING_REPORT_ID/acknowledge" > /dev/null

echo "==> Seeding a recovered outage on db-server-01..."
# Feeds the agent detail Overview's run strip, which draws one cell per run
# rather than reducing a window of days to a percentage. Three *consecutive*
# failures followed by successes is the case the strip exists to show: the
# same three failures scattered across the window would be a standing
# problem, while contiguous ones are a single incident that has since
# recovered. Only the contiguous shape earns the "Incident" chip.
#
# These are also the demo's failed rows in a recent-backups preview, where
# they carry the "View error" jump to their output (see docs/agents.md):
# db-server-01 backs up hourly, so runs 4-6 hours old are still inside the
# five rows a preview shows.
#
# archive_name stays NULL, which is both true (a failed backup writes no
# archive) and load-bearing: delete_archive_records_by_names reconciles a
# synced repo by deleting reports whose archive_name is absent from a real
# `borg list`, and `archive_name = ANY(...)` never matches NULL - so unlike
# the stale-report demo these rows survive in a repo that does get synced.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -v ON_ERROR_STOP=1 <<SQL > /dev/null
INSERT INTO backup_reports
    (agent_id, repo_id, schedule_id, started_at, finished_at, status, error_message)
SELECT $DB01_ID, $REPO_HOURLY_ID, s.id,
       NOW() - (n || ' hours')::interval - interval '3 minutes',
       NOW() - (n || ' hours')::interval,
       'failed',
       'Repository lock could not be acquired' || chr(10) ||
       'borg: Failed to create/acquire the lock /backup/repos/database-hourly/lock.exclusive'
FROM generate_series(4, 6) AS n
CROSS JOIN LATERAL (
    SELECT id FROM schedules WHERE repo_id = $REPO_HOURLY_ID ORDER BY id LIMIT 1
) s;
SQL

# The strip's Incident chip only appears when the failures are contiguous, so
# a partial insert would silently demote the scenario to "scattered" and the
# demo would stop covering the thing it was added for.
INCIDENT_COUNT=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc \
    "SELECT COUNT(*) FROM backup_reports WHERE agent_id = $DB01_ID AND status = 'failed' AND archive_name IS NULL")
if [ "$INCIDENT_COUNT" != "3" ]; then
    echo "expected exactly 3 seeded failure rows for db-server-01, found $INCIDENT_COUNT" >&2
    exit 1
fi

echo "==> Acknowledging one incident failure, so Backup Stats shows a reviewed run..."
# The dashboard's Backup Stats panel counts only failures still awaiting
# review and reports how many in the window have already been reviewed, so the
# demo needs both states inside its default 30-day range. The *oldest* run of
# the recovered incident is the one marked reviewed: it is not its target's
# latest run, so no Needs Attention finding hangs off it, and the remaining
# seeded failures stay outstanding - which is what puts the panel's "Mark
# reviewed" button on screen for the screenshot.
DB01_REVIEWED_REPORT_ID=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc \
    "SELECT id FROM backup_reports
     WHERE agent_id = $DB01_ID AND status = 'failed' AND archive_name IS NULL
     ORDER BY started_at ASC LIMIT 1")
if [ -z "$DB01_REVIEWED_REPORT_ID" ]; then
    echo "expected a db-server-01 failure to acknowledge, found none" >&2
    exit 1
fi
api POST "/api/stats/activity/$DB01_REVIEWED_REPORT_ID/acknowledge" > /dev/null

echo "==> Backfilling db-server-01's hourly run history past the first Logs page..."
# The Agent detail Logs tab paginates 50 runs at a time with a "Load more"
# button - db-server-01 is the one host whose real hourly cron would
# eventually produce that many runs, but the demo container is nowhere near
# old enough for the scheduler to have actually ticked that often. Backfill
# the history directly so the Logs tab's pagination has more than one page
# to show. archive_name stays NULL for the same reason as the incident rows
# above: it lets these survive if the repo is ever resynced.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -v ON_ERROR_STOP=1 <<SQL > /dev/null
INSERT INTO backup_reports
    (agent_id, repo_id, schedule_id, started_at, finished_at, status)
SELECT $DB01_ID, $REPO_HOURLY_ID, s.id,
       NOW() - (n || ' hours')::interval - interval '3 minutes',
       NOW() - (n || ' hours')::interval,
       'success'
FROM generate_series(10, 65) AS n
CROSS JOIN LATERAL (
    SELECT id FROM schedules WHERE repo_id = $REPO_HOURLY_ID ORDER BY id LIMIT 1
) s;
SQL

echo "==> Seeding a cancelled run on web-server-01..."
# Feeds the Schedules view's run-history strip: a cancelled run must render
# distinctly from a failed one (a muted, non-alarming bar) rather than
# falling into the same "failed" bucket, so the demo needs at least one
# backup_reports row with status = 'cancelled' to actually exercise that.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -v ON_ERROR_STOP=1 <<SQL > /dev/null
INSERT INTO backup_reports
    (agent_id, repo_id, schedule_id, started_at, finished_at, status)
VALUES (
    $WEB01_ID, $REPO_DAILY_ID, $WEB01_DAILY_SCHEDULE_ID,
    NOW() - interval '9 hours' - interval '2 minutes',
    NOW() - interval '9 hours',
    'cancelled'
);
SQL

echo "==> Seeding an in-progress run on web-server-01..."
# Feeds the agent overview's and dashboard's "Backups in progress" cards
# (see docs/agents.md, docs/dashboard.md): both derive an in-progress backup
# from a persisted backup_reports row with status = 'started', not only from
# a live WS event, so the scenario needs to exist even when nobody is
# actively watching the page when a backup happens to start.
# finished_at mirrors started_at as a placeholder, matching how
# db::insert_backup_started's own INSERT sets both to the same timestamp for
# a row that has not actually finished yet - status is what's authoritative.
# Deliberately attributed to the "Colliding daily window" schedule rather than
# WEB01_DAILY_SCHEDULE_ID (schedule 1): nothing ever resolves this row to a
# terminal status (it's not a real backup, so no agent reconnect or same-repo
# dispatch will trigger the cleanup paths that do that), so it sits at
# 'started' for the life of the demo container. Several e2e specs navigate to
# schedule 1 expecting a clean report history (see backup-lifecycle.spec.ts's
# openFirstSchedule comment) - schedule 1 must stay free of it.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -v ON_ERROR_STOP=1 <<SQL > /dev/null
INSERT INTO backup_reports
    (agent_id, repo_id, schedule_id, started_at, finished_at, status)
VALUES (
    $WEB01_ID, $REPO_DAILY_ID, $WEB01_COLLIDING_SCHEDULE_ID,
    NOW() - interval '90 seconds',
    NOW() - interval '90 seconds',
    'started'
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

echo "==> Backfilling schedule_id on imported archives..."
# Kept as the very last data-mutating step (rather than right after
# wait_for_imports()/wait_for_enrichment() return) as extra insurance now
# that wait_for_imports() itself is fixed to poll /api/repos/stats - by here,
# every other seed step that touches backup_reports (archive tagging, the
# warnings UPDATE) has already run against the same data, so the import is
# guaranteed to have landed.
#
# web-server-01's server-daily archives match both its own solo daily
# schedule (line ~265) and the multi-agent sequential schedule (line ~346),
# since both target the same repo and include this agent. A plain UPDATE ...
# FROM would pick an unspecified one of the matching schedules per row when
# more than one qualifies, so the DISTINCT ON below deterministically picks
# the lowest schedule id (the report's earliest/primary owning schedule) -
# required for the dashboard's per-schedule average-duration lookups to find
# a consistent, non-empty history.
PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -v ON_ERROR_STOP=1 <<SQL
UPDATE backup_reports br
SET schedule_id = matched.schedule_id
FROM (
    SELECT DISTINCT ON (br2.id) br2.id AS report_id, s.id AS schedule_id
    FROM backup_reports br2
    JOIN schedules s ON s.repo_id = br2.repo_id
    JOIN schedule_targets st ON st.schedule_id = s.id AND st.agent_id = br2.agent_id
    WHERE br2.schedule_id IS NULL
      AND s.enabled = true
      AND s.name NOT IN ('Offline agent due soon', 'Colliding daily window')
    ORDER BY br2.id, s.id
) matched
WHERE br.id = matched.report_id;
SQL

# Fail loudly here rather than leaving the dashboard ETA e2e test to fail with
# a confusing "left" timeout 15+ minutes later. The dashboard's per-schedule
# average-duration lookup (frontend/e2e/fixtures.ts's
# mockRunningBackupOperation and dashboard.spec.ts) hardcodes schedule_id=1
# for web-server-01's server-daily archives, so the backfill above must land
# on exactly that id for all 14 of them.
WEB01_SERVER_DAILY_SCHEDULE_IDS=$(PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -tAc \
    "SELECT COALESCE(br.schedule_id::text, 'NULL') || ':' || COUNT(*) FROM backup_reports br \
     JOIN agents a ON a.id = br.agent_id JOIN repos r ON r.id = br.repo_id \
     WHERE a.hostname = 'web-server-01' AND r.name = 'server-daily' AND br.archive_name IS NOT NULL \
     GROUP BY br.schedule_id ORDER BY br.schedule_id")
if [ "$WEB01_SERVER_DAILY_SCHEDULE_IDS" != "1:14" ]; then
    echo "expected all 14 web-server-01/server-daily imported archives to have schedule_id=1, found: $WEB01_SERVER_DAILY_SCHEDULE_IDS" >&2
    exit 1
fi

echo "==> Demo data seeded successfully."
