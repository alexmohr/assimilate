-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- A schedule's backups can read from filesystem snapshots (ZFS, btrfs, LVM)
-- of their sources instead of the live files. The mode mirrors
-- shared::snapshot::FsSnapshotMode: 'off' reads the live files, 'required'
-- fails a run whose sources cannot all be snapshotted, 'best_effort' backs
-- such a source up live and ends the run with a warning.
ALTER TABLE schedules
    ADD COLUMN fs_snapshot TEXT NOT NULL DEFAULT 'off'
        CHECK (fs_snapshot IN ('off', 'required', 'best_effort')),
    -- Size of an LVM snapshot, in percent of the volume it snapshots.
    ADD COLUMN lvm_snapshot_percent SMALLINT NOT NULL DEFAULT 10
        CHECK (lvm_snapshot_percent BETWEEN 1 AND 100);

-- The agent records its snapshot steps on the run's timeline.
ALTER TABLE backup_run_events DROP CONSTRAINT backup_run_events_event_type_check;

ALTER TABLE backup_run_events
    ADD CONSTRAINT backup_run_events_event_type_check
    CHECK (event_type IN (
        'reachability_check', 'wake_sent', 'wake_unavailable', 'host_online',
        'host_unreachable', 'agent_start_sent', 'agent_connected',
        'agent_stop_sent', 'agent_stopped', 'shutdown_sent', 'host_offline',
        'snapshot_created', 'snapshot_removed', 'snapshot_failed'
    )) NOT VALID;
