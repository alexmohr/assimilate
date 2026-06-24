// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

export interface AgentRow {
  id: number
  hostname: string
  display_name: string | null
  agent_version?: string | null
  agent_git_sha?: string | null
  agent_build_time?: string | null
  agent_commit_count?: number | null
  created_at?: string
  last_seen_at?: string | null
  is_connected?: boolean
  is_imported?: boolean
  is_hidden?: boolean
  supports_restart?: boolean
  restart_unavailable_reason?: string | null
  default_backup_paths?: string[]
  default_exclude_patterns?: string[]
  default_pre_backup_commands?: string
  default_post_backup_commands?: string
}
