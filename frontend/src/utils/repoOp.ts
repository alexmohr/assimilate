// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ActiveRepoOp } from '../types/repo'

/** Human label for the borg operation a repository is currently running. */
export function repoOpLabel(op: ActiveRepoOp): string {
  const queued = op.queued && op.queued > 0 ? ` (+${op.queued} queued)` : ''
  switch (op.kind) {
    case 'agent_backup':
      return `Agent backup in progress by ${op.actor}${queued}`
    case 'server_sync':
      return `Server sync in progress${queued}`
    case 'break_lock':
      return `Break-lock in progress${queued}`
    case 'delete_archive':
      return `Deleting archive (started by ${op.actor})${queued}`
    case 'agent_check':
      return `Integrity check in progress by ${op.actor}${queued}`
    case 'agent_verify':
      return `Verify in progress by ${op.actor}${queued}`
    case 'compact_repo':
      return `Compacting repository (started by ${op.actor})${queued}`
  }
}
