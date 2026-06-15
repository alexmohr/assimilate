// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

export interface ReportRow {
  id: number
  agent_id?: number
  machine_id?: number
  repo_id: number
  started_at: string
  finished_at: string
  status: string
  original_size: number
  compressed_size: number
  deduplicated_size: number
  files_processed: number
  duration_secs: number
  error_message: string | null
  warnings?: string[]
  borg_version: string | null
  archive_name?: string | null
  borg_command?: string | null
}
