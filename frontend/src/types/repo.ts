// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  RepoOpKind,
  ActiveRepoOp as GeneratedActiveRepoOp,
  RepoResponse,
  RepoWithStatsResponse,
} from './generated'

export type { RepoOpKind }
export type ActiveRepoOp = GeneratedActiveRepoOp
export type Repo = RepoResponse
export type RepoWithStats = RepoWithStatsResponse
