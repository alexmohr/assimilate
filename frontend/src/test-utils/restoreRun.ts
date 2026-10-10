// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { RestoreRun } from '../types/generated'

/** A restore onto `web-01`, running, for tests to adjust. */
export function makeRestoreRun(overrides: Partial<RestoreRun> = {}): RestoreRun {
  return {
    id: '7c9e6679-7425-40de-944b-e07fc1f90ae7',
    agent_id: 3,
    hostname: 'web-01',
    repo_id: 5,
    repo_name: 'nas-daily',
    archive_name: 'web-01-2026-05-30',
    paths: ['etc/hosts'],
    target_path: '/restore',
    status: 'running',
    files_restored: null,
    error_message: null,
    requested_by: 'admin',
    created_at: '2026-05-31T10:00:00Z',
    started_at: '2026-05-31T10:00:00Z',
    finished_at: null,
    ...overrides,
  }
}
