// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

export type RepoOpKind = 'agent_backup' | 'server_sync' | 'break_lock' | 'delete_archive'

export interface ActiveRepoOp {
  kind: RepoOpKind
  actor: string
  started_at: string
  queued?: number
}

export interface RepoWithStats {
  id: number
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  ssh_host_key: string | null
  compression: string
  encryption: string
  enabled: boolean
  importing: boolean
  import_error: string | null
  import_progress: number
  import_total: number
  import_status_message: string | null
  sync_schedule?: string | null
  last_synced_at?: string | null
  archive_count: number
  last_backup_at: string | null
  total_original_size: number
  total_compressed_size: number
  total_deduplicated_size: number
  client_count: number
  unmatched_count: number
  relocation_pending?: boolean
  last_op_kind?: string | null
  last_op_at?: string | null
  last_op_by?: string | null
  current_op?: ActiveRepoOp | null
}
