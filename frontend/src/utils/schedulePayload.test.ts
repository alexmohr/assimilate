// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  agentOverridePayload,
  primaryRepoId,
  repoTargetsProblem,
  scheduleFormPayload,
} from './schedulePayload'
import { DEFAULT_SCHEDULE_FORM_STATE } from '../types/scheduleForm'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'

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

function overrides(patch: Partial<ScheduleAgentOverrides> = {}): ScheduleAgentOverrides {
  return {
    usePerHostExcludes: false,
    perHostExcludes: {},
    usePerHostFileChangePatterns: false,
    perHostFileChangePatterns: {},
    usePerAgentCmds: false,
    perAgentPreCmds: {},
    perAgentPostCmds: {},
    ...patch,
  }
}

describe('agentOverridePayload', () => {
  it('sends nothing while every override is off', () => {
    expect(agentOverridePayload(overrides(), [1, 2])).toEqual({})
  })

  it('clears the shared excludes and sends one entry per selected host', () => {
    const payload = agentOverridePayload(
      overrides({ usePerHostExcludes: true, perHostExcludes: { 1: '/var/cache' } }),
      [1, 2],
    )

    expect(payload.exclude_patterns_raw).toBe('')
    expect(payload.exclude_patterns_per_agent).toEqual([
      { agent_id: 1, raw_text: '/var/cache' },
      // A host the operator left blank still has to be sent, or it keeps
      // whatever it had before.
      { agent_id: 2, raw_text: '' },
    ])
    expect(payload.file_change_patterns_per_agent).toBeUndefined()
    expect(payload.commands_per_agent).toBeUndefined()
  })

  it('clears the shared file-change patterns the same way', () => {
    const payload = agentOverridePayload(
      overrides({
        usePerHostFileChangePatterns: true,
        perHostFileChangePatterns: { 3: '/etc/** warn' },
      }),
      [3],
    )

    expect(payload.file_change_patterns_raw).toBe('')
    expect(payload.file_change_patterns_per_agent).toEqual([
      { agent_id: 3, raw_text: '/etc/** warn' },
    ])
  })

  it('drops blank per-agent commands and clears the shared hooks', () => {
    const payload = agentOverridePayload(
      overrides({
        usePerAgentCmds: true,
        perAgentPreCmds: {
          1: [
            { command: 'systemctl stop nginx', timeout_seconds: null },
            { command: '  ', timeout_seconds: null },
          ],
        },
        perAgentPostCmds: { 1: [{ command: 'systemctl start nginx', timeout_seconds: 30 }] },
      }),
      [1],
    )

    expect(payload.pre_backup_commands).toEqual([])
    expect(payload.post_backup_commands).toEqual([])
    expect(payload.commands_per_agent).toEqual([
      {
        agent_id: 1,
        pre_backup_commands: [{ command: 'systemctl stop nginx', timeout_seconds: null }],
        post_backup_commands: [{ command: 'systemctl start nginx', timeout_seconds: 30 }],
      },
    ])
  })
})

describe('repoTargetsProblem', () => {
  it('accepts a list with a required target', () => {
    expect(repoTargetsProblem([{ repo_id: 1, required: true }])).toBeNull()
    expect(
      repoTargetsProblem([
        { repo_id: 1, required: false },
        { repo_id: 2, required: true },
      ]),
    ).toBeNull()
  })

  it('names an empty list', () => {
    expect(repoTargetsProblem([])).toBe('at least one repository')
  })

  /** Without a required target a run could report success having written nothing. */
  it('names a list with nothing required', () => {
    expect(repoTargetsProblem([{ repo_id: 1, required: false }])).toBe(
      'at least one required repository',
    )
  })
})

describe('primaryRepoId', () => {
  /** The server's `primary_target` picks the first *required* target, so a
      list that starts with a best-effort one is where the two rules diverge -
      and the case the old `targets[0]` version got wrong. */
  it('takes the first required target, not the first written one', () => {
    expect(
      primaryRepoId([
        { repo_id: 20, required: false },
        { repo_id: 21, required: true },
      ]),
    ).toBe(21)
  })

  it('falls back to the first target when none is required', () => {
    expect(
      primaryRepoId([
        { repo_id: 20, required: false },
        { repo_id: 21, required: false },
      ]),
    ).toBe(20)
  })

  it('takes the only target of a single-target schedule', () => {
    expect(primaryRepoId([{ repo_id: 20, required: true }])).toBe(20)
  })
})
