// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  RepoOpKind,
  ActiveRepoOp as GeneratedActiveRepoOp,
  RepoWithStatsResponse,
} from './generated'

export type { RepoOpKind }
export type ActiveRepoOp = GeneratedActiveRepoOp
export type RepoWithStats = Omit<
  RepoWithStatsResponse,
  | 'id'
  | 'archive_count'
  | 'agent_count'
  | 'unmatched_count'
  | 'total_original_size'
  | 'total_compressed_size'
  | 'total_deduplicated_size'
  | 'import_progress'
  | 'import_total'
  | 'ssh_port'
> & {
  id: number
  archive_count: number
  agent_count: number
  unmatched_count: number
  total_original_size: number
  total_compressed_size: number
  total_deduplicated_size: number
  import_progress: number
  import_total: number
  ssh_port: number
}
