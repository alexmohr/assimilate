-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- A repository host is the machine borg writes to. Everything that is a fact
-- about that machine rather than about one repository on it - its address,
-- the SSH host key it presents, how to wake it, whether it sleeps - used to be
-- repeated on every repository, so three repositories on one NAS could
-- disagree about whether that NAS sleeps, which MAC address wakes it, or
-- which key it presents. It now lives here, once.
--
-- One hostname is one host, with exactly one SSH port and one pinned key: the
-- storage quota in server_quotas is already keyed by hostname, and a host that
-- could span several ports would split that quota. A repository keeps what is
-- really its own: the user it logs in as and its path.
CREATE TABLE repo_hosts (
    id BIGSERIAL PRIMARY KEY,
    ssh_host TEXT NOT NULL UNIQUE CHECK (ssh_host <> ''),
    ssh_port INTEGER NOT NULL DEFAULT 22 CHECK (ssh_port BETWEEN 1 AND 65535),
    ssh_host_key TEXT,
    wake_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    wake_mac_address TEXT,
    wake_broadcast_address TEXT,
    wake_timeout_seconds INTEGER NOT NULL DEFAULT 180 CHECK (wake_timeout_seconds > 0),
    shutdown_after_backup BOOLEAN NOT NULL DEFAULT FALSE,
    intermittent BOOLEAN NOT NULL DEFAULT FALSE,
    catch_up_recheck_minutes INTEGER NOT NULL DEFAULT 15 CHECK (catch_up_recheck_minutes > 0),
    catch_up_give_up_minutes INTEGER NOT NULL DEFAULT 0 CHECK (catch_up_give_up_minutes >= 0),
    CONSTRAINT repo_hosts_wake_mac_format CHECK (
        wake_mac_address IS NULL OR wake_mac_address ~* '^([0-9A-F]{2}:){5}[0-9A-F]{2}$'
    ),
    CONSTRAINT repo_hosts_wake_requires_mac CHECK (NOT wake_enabled OR wake_mac_address IS NOT NULL),
    CONSTRAINT repo_hosts_shutdown_requires_mac CHECK (
        NOT shutdown_after_backup OR wake_mac_address IS NOT NULL
    )
);

-- The migration writes what it had to decide into the Activity Log, so the
-- event type has to exist before the rows below are inserted.
ALTER TABLE system_events DROP CONSTRAINT system_events_event_type_check;

ALTER TABLE system_events
    ADD CONSTRAINT system_events_event_type_check
    CHECK (event_type IN (
        'repo_sync', 'repo_sync_slow', 'repo_sync_failed', 'repo_sync_cancelled',
        'archive_delete_failed', 'archive_compact_failed', 'account_locked',
        'auth_failed', 'security_violation', 'schedule_auto_disabled',
        'schedule_reenabled', 'schedule_catch_up', 'backup_skipped_agent_offline',
        'backup_skipped_repo_offline', 'schedule_catch_up_abandoned',
        'backup_failed_agent_offline', 'repo_host_migrated'
    )) NOT VALID;

-- Step 1: group. Repositories that reach one machine belong to one host. Two
-- repositories are the same machine when they use the same hostname, or when
-- they pinned the same host key on the same port - a matching key proves it is
-- the same SSH server whatever name each one used (nas, nas.lan, an IP). The
-- groups are the connected components of that relation, so nas -> nas.lan ->
-- 192.168.1.4 chains into one host even when no single repository links the
-- first name to the last.
CREATE TEMPORARY TABLE repo_host_groups ON COMMIT DROP AS
WITH RECURSIVE same_machine AS (
    SELECT a.id AS from_id, b.id AS to_id
    FROM repos a
    JOIN repos b ON a.ssh_host = b.ssh_host
        OR (a.ssh_host_key IS NOT NULL
            AND a.ssh_host_key = b.ssh_host_key
            AND a.ssh_port = b.ssh_port)
),
reach (root_id, repo_id) AS (
    SELECT id, id FROM repos
    UNION
    SELECT reach.root_id, same_machine.to_id
    FROM reach
    JOIN same_machine ON same_machine.from_id = reach.repo_id
)
SELECT repo_id, MIN(root_id) AS group_id
FROM reach
GROUP BY repo_id;

-- The most recent write per repository, the tie-breaker for which name wins.
CREATE TEMPORARY TABLE repo_last_write ON COMMIT DROP AS
SELECT r.id AS repo_id, MAX(br.finished_at) AS last_write
FROM repos r
LEFT JOIN backup_reports br ON br.repo_id = r.id
GROUP BY r.id;

-- Step 2: name. The host takes the hostname most of its repositories used; a
-- tie goes to the name used by the most recently written repository.
CREATE TEMPORARY TABLE repo_host_names ON COMMIT DROP AS
SELECT DISTINCT ON (g.group_id) g.group_id, r.ssh_host
FROM repo_host_groups g
JOIN repos r ON r.id = g.repo_id
JOIN repo_last_write lw ON lw.repo_id = r.id
GROUP BY g.group_id, r.ssh_host
ORDER BY g.group_id, COUNT(*) DESC, MAX(lw.last_write) DESC NULLS LAST, r.ssh_host;

-- ... and the port most of them used, with the same tie-breaker.
CREATE TEMPORARY TABLE repo_host_ports ON COMMIT DROP AS
SELECT DISTINCT ON (g.group_id) g.group_id, r.ssh_port
FROM repo_host_groups g
JOIN repos r ON r.id = g.repo_id
JOIN repo_last_write lw ON lw.repo_id = r.id
GROUP BY g.group_id, r.ssh_port
ORDER BY g.group_id, COUNT(*) DESC, MAX(lw.last_write) DESC NULLS LAST, r.ssh_port;

-- The key most repositories on the chosen port pinned; a tie goes to the
-- newest repository, the one whose key was accepted last.
CREATE TEMPORARY TABLE repo_host_keys ON COMMIT DROP AS
SELECT DISTINCT ON (g.group_id) g.group_id, r.ssh_host_key
FROM repo_host_groups g
JOIN repos r ON r.id = g.repo_id
JOIN repo_host_ports p ON p.group_id = g.group_id AND p.ssh_port = r.ssh_port
WHERE r.ssh_host_key IS NOT NULL
GROUP BY g.group_id, r.ssh_host_key
ORDER BY g.group_id, COUNT(*) DESC, MAX(r.id) DESC;

-- The wake address comes from the newest repository that wakes the host, or
-- failing that the newest one that has an address at all (shutting down needs
-- one too).
CREATE TEMPORARY TABLE repo_host_wake ON COMMIT DROP AS
SELECT DISTINCT ON (g.group_id) g.group_id, r.wake_mac_address, r.wake_broadcast_address
FROM repo_host_groups g
JOIN repos r ON r.id = g.repo_id
WHERE r.wake_mac_address IS NOT NULL
ORDER BY g.group_id, r.wake_enabled DESC, r.id DESC;

-- Step 3: fold. When repositories on one host disagree, each setting takes the
-- value that changes nothing for any of them for the worse:
--
-- * waking and "not always online" are on if any repository had them on, so
--   no backup that used to wake the host, or be a skip, stops doing so;
-- * the re-check interval is the shortest and the wake timeout the longest;
-- * the give-up window is the longest, with 0 (wait indefinitely) winning;
-- * shutting down is on only if every repository had it on - never power down
--   a machine that one repository's owner expects to stay up.
INSERT INTO repo_hosts (
    ssh_host, ssh_port, ssh_host_key, wake_enabled, wake_mac_address, wake_broadcast_address,
    wake_timeout_seconds, shutdown_after_backup, intermittent, catch_up_recheck_minutes,
    catch_up_give_up_minutes
)
SELECT
    n.ssh_host,
    p.ssh_port,
    k.ssh_host_key,
    bool_or(r.wake_enabled) AND w.wake_mac_address IS NOT NULL,
    w.wake_mac_address,
    w.wake_broadcast_address,
    MAX(r.wake_timeout_seconds),
    bool_and(r.shutdown_after_backup) AND w.wake_mac_address IS NOT NULL,
    bool_or(r.intermittent),
    MIN(r.catch_up_recheck_minutes),
    CASE WHEN bool_or(r.catch_up_give_up_minutes = 0) THEN 0
        ELSE MAX(r.catch_up_give_up_minutes) END
FROM repo_host_groups g
JOIN repos r ON r.id = g.repo_id
JOIN repo_host_names n ON n.group_id = g.group_id
JOIN repo_host_ports p ON p.group_id = g.group_id
LEFT JOIN repo_host_keys k ON k.group_id = g.group_id
LEFT JOIN repo_host_wake w ON w.group_id = g.group_id
GROUP BY g.group_id, n.ssh_host, p.ssh_port, k.ssh_host_key, w.wake_mac_address,
    w.wake_broadcast_address;

ALTER TABLE repos ADD COLUMN repo_host_id BIGINT REFERENCES repo_hosts (id) ON DELETE RESTRICT;

UPDATE repos r
SET repo_host_id = h.id
FROM repo_host_groups g
JOIN repo_host_names n ON n.group_id = g.group_id
JOIN repo_hosts h ON h.ssh_host = n.ssh_host
WHERE g.repo_id = r.id;

-- Everything the fold could not decide without changing where or how a
-- repository connects is written to the Activity Log, named per repository,
-- so an admin can see what moved and fix what should not have. None of it is
-- silently trusted: a repository now pointed at another port or key fails to
-- connect rather than talking to a different SSH server.
INSERT INTO system_events (event_type, hostname, message)
SELECT 'repo_host_migrated', h.ssh_host,
    format(
        'Repository ''%s'' was reached as %s and now uses its repository host %s, which the '
        || 'same SSH host key identifies. If an agent cannot resolve %s, edit the host''s '
        || 'hostname.',
        r.name, r.ssh_host, h.ssh_host, h.ssh_host
    )
FROM repos r
JOIN repo_hosts h ON h.id = r.repo_host_id
WHERE r.ssh_host <> h.ssh_host;

INSERT INTO system_events (event_type, hostname, message)
SELECT 'repo_host_migrated', h.ssh_host,
    format(
        'Repository ''%s'' used SSH port %s, but its repository host %s uses port %s, the one '
        || 'most of its repositories use. Fix the port on the host, or move the repository to '
        || 'another host.',
        r.name, r.ssh_port, h.ssh_host, h.ssh_port
    )
FROM repos r
JOIN repo_hosts h ON h.id = r.repo_host_id
WHERE r.ssh_port <> h.ssh_port;

INSERT INTO system_events (event_type, hostname, message)
SELECT 'repo_host_migrated', h.ssh_host,
    format(
        'Repository ''%s'' had pinned a different SSH host key than its repository host %s '
        || 'keeps. Connections that present the other key are refused until an admin accepts '
        || 'it on the host.',
        r.name, h.ssh_host
    )
FROM repos r
JOIN repo_hosts h ON h.id = r.repo_host_id
WHERE r.ssh_host_key IS NOT NULL
    AND h.ssh_host_key IS NOT NULL
    AND r.ssh_host_key <> h.ssh_host_key;

INSERT INTO system_events (event_type, hostname, message)
SELECT 'repo_host_migrated', h.ssh_host,
    format(
        'Repository ''%s'' had a different Wake-on-LAN address than its repository host %s '
        || 'keeps (%s). Check the address on the host.',
        r.name, h.ssh_host, h.wake_mac_address
    )
FROM repos r
JOIN repo_hosts h ON h.id = r.repo_host_id
WHERE r.wake_mac_address IS NOT NULL
    AND h.wake_mac_address IS NOT NULL
    AND upper(r.wake_mac_address) <> upper(h.wake_mac_address);

-- A storage quota set on a name that lost the vote moves to the host when the
-- host has none of its own; otherwise it is dropped and logged. Only one of a
-- host's losing names can move - the first, by name, so the choice is stable.
CREATE TEMPORARY TABLE repo_host_quota_moves ON COMMIT DROP AS
SELECT DISTINCT ON (h.id) h.id AS repo_host_id, sq.ssh_host AS from_host, h.ssh_host AS to_host
FROM repo_hosts h
JOIN repos r ON r.repo_host_id = h.id AND r.ssh_host <> h.ssh_host
JOIN server_quotas sq ON sq.ssh_host = r.ssh_host
WHERE NOT EXISTS (SELECT 1 FROM server_quotas own WHERE own.ssh_host = h.ssh_host)
ORDER BY h.id, sq.ssh_host;

UPDATE server_quotas sq
SET ssh_host = m.to_host
FROM repo_host_quota_moves m
WHERE sq.ssh_host = m.from_host;

INSERT INTO system_events (event_type, hostname, message)
SELECT DISTINCT 'repo_host_migrated', h.ssh_host,
    format(
        'The storage quota set on %s was removed: its repositories now use the repository host '
        || '%s, which keeps its own quota.',
        sq.ssh_host, h.ssh_host
    )
FROM repo_hosts h
JOIN repos r ON r.repo_host_id = h.id AND r.ssh_host <> h.ssh_host
JOIN server_quotas sq ON sq.ssh_host = r.ssh_host;

DELETE FROM server_quotas sq
USING repos r
JOIN repo_hosts h ON h.id = r.repo_host_id
WHERE sq.ssh_host = r.ssh_host AND r.ssh_host <> h.ssh_host;

ALTER TABLE repos ALTER COLUMN repo_host_id SET NOT NULL;

CREATE INDEX idx_repos_repo_host_id ON repos (repo_host_id);

-- The constraints on these columns (repos_wake_mac_format,
-- repos_wake_requires_mac, repos_shutdown_requires_mac and the catch-up range
-- checks) go with them.
ALTER TABLE repos
    DROP COLUMN ssh_host,
    DROP COLUMN ssh_port,
    DROP COLUMN ssh_host_key,
    DROP COLUMN wake_enabled,
    DROP COLUMN wake_mac_address,
    DROP COLUMN wake_broadcast_address,
    DROP COLUMN wake_timeout_seconds,
    DROP COLUMN shutdown_after_backup,
    DROP COLUMN intermittent,
    DROP COLUMN catch_up_recheck_minutes,
    DROP COLUMN catch_up_give_up_minutes;
