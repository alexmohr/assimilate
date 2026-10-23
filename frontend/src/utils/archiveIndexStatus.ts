// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/** Where an archive's content index is, as the server reports it. */
export type ArchiveIndexStatus = 'pending' | 'indexing' | 'done' | 'failed'

/** The server's index status string, narrowed; anything unknown is still pending. */
export function normalizeIndexStatus(status: string): ArchiveIndexStatus {
  if (status === 'done') return 'done'
  if (status === 'failed') return 'failed'
  if (status === 'indexing') return 'indexing'
  return 'pending'
}
