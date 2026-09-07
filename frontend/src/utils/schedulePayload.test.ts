// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { scheduleFormPayload } from './schedulePayload'
import { DEFAULT_SCHEDULE_FORM_STATE } from '../types/scheduleForm'
import type { ScheduleFormState } from '../types/scheduleForm'

function form(overrides: Partial<ScheduleFormState> = {}): ScheduleFormState {
  return { ...DEFAULT_SCHEDULE_FORM_STATE, ...overrides }
}

describe('scheduleFormPayload', () => {
  it('renames the form fields the API spells differently', () => {
    const payload = scheduleFormPayload(
      form({ exclude_patterns: '*.tmp', file_change_patterns: '/etc/** warn' }),
    )

    expect(payload.exclude_patterns_raw).toBe('*.tmp')
    expect(payload.file_change_patterns_raw).toBe('/etc/** warn')
  })

  it('splits backup paths into a list, dropping blank lines', () => {
    const payload = scheduleFormPayload(form({ backup_sources: '/etc\n\n  /srv  \n' }))

    expect(payload.backup_sources).toEqual(['/etc', '/srv'])
  })

  it('drops hook commands with no command left in them', () => {
    const payload = scheduleFormPayload(
      form({
        pre_backup_commands: [
          { command: 'systemctl stop nginx', timeout_seconds: null },
          { command: '   ', timeout_seconds: null },
        ],
        post_backup_commands: [{ command: '', timeout_seconds: null }],
      }),
    )

    expect(payload.pre_backup_commands).toEqual([
      { command: 'systemctl stop nginx', timeout_seconds: null },
    ])
    expect(payload.post_backup_commands).toEqual([])
  })

  it('carries the retention counts and thresholds through unchanged', () => {
    const payload = scheduleFormPayload(
      form({ keep_hourly: 12, keep_yearly: 3, missed_backup_threshold: 5, rate_limit_kbps: 2048 }),
    )

    expect(payload.keep_hourly).toBe(12)
    expect(payload.keep_yearly).toBe(3)
    expect(payload.missed_backup_threshold).toBe(5)
    expect(payload.rate_limit_kbps).toBe(2048)
  })
})
