// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { RepoWithStats } from '../types/repo'

/**
 * One repository fixture for the specs that render one. The header and the
 * settings pane are two components over the same row, so they were carrying
 * two copies of it that could disagree about the same repository.
 */
const REPO = {
  id: 12,
  name: 'server-daily',
  repo_path: '/backup/repos/server-daily',
  ssh_user: 'borg',
  ssh_host: 'backup.example.com',
  ssh_port: 22,
  ssh_host_key: 'ssh-ed25519 AAAAKNOWN',
  compression: 'zstd,6',
  encryption: 'repokey-blake2',
  enabled: true,
  importing: false,
  import_error: null,
  import_progress: 0,
  import_total: 0,
  sync_schedule: null,
  agent_count: 3,
  archive_count: 30,
  total_deduplicated_size: 1024,
  total_original_size: 4096,
  last_backup_at: '2026-03-01T02:00:00Z',
  last_op_kind: 'agent_backup',
  last_op_at: '2026-03-01T02:00:00Z',
  last_op_by: 'web-01',
} as unknown as RepoWithStats

export function repoFixture(overrides: Partial<RepoWithStats> = {}): RepoWithStats {
  return { ...REPO, ...overrides }
}
