// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  RepoOpKind,
  ActiveRepoOp as GeneratedActiveRepoOp,
  RepoWithStatsResponse,
} from './generated'

export type { RepoOpKind }
export type ActiveRepoOp = GeneratedActiveRepoOp
export type RepoWithStats = RepoWithStatsResponse
