// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ReportResponse } from './generated'

export type ReportRow = Omit<
  ReportResponse,
  | 'id'
  | 'agent_id'
  | 'repo_id'
  | 'schedule_id'
  | 'original_size'
  | 'compressed_size'
  | 'deduplicated_size'
  | 'files_processed'
  | 'duration_secs'
> & {
  id: number
  agent_id: number
  repo_id: number
  schedule_id: number | null
  original_size: number
  compressed_size: number
  deduplicated_size: number
  files_processed: number
  duration_secs: number
}
