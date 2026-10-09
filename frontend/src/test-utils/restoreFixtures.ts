// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { RestoreResponse } from '../types/generated'

/** A restore onto `web-server-01` as the API returns it, finished by default. */
export function restoreFixture(overrides: Partial<RestoreResponse> = {}): RestoreResponse {
  return {
    id: 1,
    repo_id: 1,
    archive_name: 'web-server-01-2026-05-30T12:00:00',
    paths: ['etc/hosts'],
    target_path: '/tmp/restore',
    agent_id: 1,
    hostname: 'web-server-01',
    status: 'succeeded',
    files_restored: 1,
    error_message: null,
    requested_by: 'admin',
    created_at: '2026-10-09T12:00:00Z',
    started_at: '2026-10-09T12:00:01Z',
    finished_at: '2026-10-09T12:00:02Z',
    ...overrides,
  }
}

/** A restore that ended in failure, with the agent's reason. */
export function failedRestoreFixture(errorMessage: string): RestoreResponse {
  return restoreFixture({ status: 'failed', files_restored: null, error_message: errorMessage })
}
