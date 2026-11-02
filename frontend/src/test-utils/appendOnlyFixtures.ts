// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { AppendOnlyStatusResponse } from '../types/generated'

/**
 * An enabled, healthy append-only status for the specs of the append-only
 * panes: two sibling repositories, both keys reaching the host, a weekly
 * maintenance schedule and nothing queued.
 */
const STATUS: AppendOnlyStatusResponse = {
  enabled: true,
  login: {
    ssh_host: 'backup.lan',
    ssh_user: 'borg',
    sibling_repos: [
      { id: 2, name: 'laptops' },
      { id: 3, name: 'photos' },
    ],
  },
  installed_at: '2026-10-28T09:00:00Z',
  check: {
    checked_at: '2026-10-28T09:01:00Z',
    backup_key_ok: true,
    maintenance_key_ok: true,
    restriction_in_place: true,
    error: null,
  },
  backup_key: {
    fingerprint: 'SHA256:backupfp',
    authorized_keys_line: 'command="borg serve --append-only",restrict ssh-ed25519 AAAAbackup',
  },
  maintenance_key: {
    fingerprint: 'SHA256:maintfp',
    authorized_keys_line: 'ssh-ed25519 AAAAmaintenance assimilate-maintenance',
  },
  maintenance: {
    cron_expression: '30 4 * * 0',
    check_before_prune: true,
    next_run_at: '2026-11-08T04:30:00Z',
    last_run_at: '2026-11-01T04:30:00Z',
    stopped: null,
  },
  pending_deletions: [],
  recent_runs: [],
  setup_failure: null,
}

export function appendOnlyStatusFixture(
  overrides: Partial<AppendOnlyStatusResponse> = {},
): AppendOnlyStatusResponse {
  return { ...STATUS, ...overrides }
}
