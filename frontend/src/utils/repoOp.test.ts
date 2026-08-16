// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import type { ActiveRepoOp, RepoOpKind } from '../types/repo'
import { repoOpLabel } from './repoOp'

function op(kind: RepoOpKind, overrides: Partial<ActiveRepoOp> = {}): ActiveRepoOp {
  return {
    kind,
    actor: 'web-01',
    started_at: '2026-03-01T02:00:00Z',
    queued: 0,
    ...overrides,
  }
}

describe('repoOpLabel', () => {
  // One case per RepoOpKind arm: adding a kind to the generated union without
  // giving it a label fails to compile here rather than rendering blank.
  it.each([
    ['agent_backup', 'Agent backup in progress by web-01'],
    ['server_sync', 'Server sync in progress'],
    ['break_lock', 'Break-lock in progress'],
    ['delete_archive', 'Deleting archive (started by web-01)'],
    ['agent_check', 'Integrity check in progress by web-01'],
    ['agent_verify', 'Verify in progress by web-01'],
    ['compact_repo', 'Compacting repository (started by web-01)'],
  ] as const)('labels %s', (kind, label) => {
    expect(repoOpLabel(op(kind))).toBe(label)
  })

  it('appends the queue depth when other operations are waiting', () => {
    expect(repoOpLabel(op('server_sync', { queued: 3 }))).toBe(
      'Server sync in progress (+3 queued)',
    )
    expect(repoOpLabel(op('agent_backup', { queued: 1 }))).toBe(
      'Agent backup in progress by web-01 (+1 queued)',
    )
  })

  it('leaves the queue suffix off when nothing is waiting', () => {
    expect(repoOpLabel(op('break_lock', { queued: 0 }))).toBe('Break-lock in progress')
  })
})
