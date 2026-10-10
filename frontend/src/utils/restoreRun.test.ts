// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import type { RestoreRunStatus } from '../types/generated'
import {
  isRestoreFinished,
  restoreScope,
  restoreStatusLabel,
  restoreStatusTone,
} from './restoreRun'

const ALL: RestoreRunStatus[] = ['pending', 'running', 'success', 'failed', 'cancelled']

describe('restoreRun', () => {
  it('counts only an ended restore as finished', () => {
    expect(ALL.filter(isRestoreFinished)).toEqual(['success', 'failed', 'cancelled'])
  })

  it('labels every status', () => {
    expect(ALL.map(restoreStatusLabel)).toEqual([
      'Waiting for agent',
      'Restoring',
      'Restored',
      'Failed',
      'Cancelled',
    ])
  })

  it('tones every status', () => {
    expect(ALL.map(restoreStatusTone)).toEqual(['neutral', 'info', 'success', 'danger', 'neutral'])
  })

  it('names the paths, or the whole archive when there are none', () => {
    expect(restoreScope({ paths: ['etc/hosts', 'etc/fstab'] })).toBe('etc/hosts, etc/fstab')
    expect(restoreScope({ paths: [] })).toBe('the whole archive')
  })
})
