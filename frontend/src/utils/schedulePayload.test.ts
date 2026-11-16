// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  agentOverridePayload,
  ownPathsPayload,
  primaryRepoId,
  repoTargetsProblem,
  scheduleFormPayload,
  sourcesProblem,
} from './schedulePayload'
import { DEFAULT_SCHEDULE_FORM_STATE } from '../types/scheduleForm'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'

function form(overrides: Partial<ScheduleFormState> = {}): ScheduleFormState {
  return { ...DEFAULT_SCHEDULE_FORM_STATE, ...overrides }
}

describe('scheduleFormPayload', () => {
  it('renames the form fields the API spells differently', () => {
    const payload = scheduleFormPayload(
      form({
        exclude_patterns: '*.tmp',
        include_patterns: '/home/keep',
        file_change_patterns: '/etc/** warn',
      }),
    )

    expect(payload.exclude_patterns_raw).toBe('*.tmp')
    expect(payload.include_patterns_raw).toBe('/home/keep')
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

  it("sends where the files come from and whether the hosts' commands run", () => {
    expect(scheduleFormPayload(form())).toMatchObject({
      files_mode: 'host',
      run_host_commands: true,
    })
    expect(
      scheduleFormPayload(form({ files_mode: 'off', run_host_commands: false })),
    ).toMatchObject({ files_mode: 'off', run_host_commands: false })
  })
})

function overrides(patch: Partial<ScheduleAgentOverrides> = {}): ScheduleAgentOverrides {
  return {
    usePerHostExcludes: false,
    perHostExcludes: {},
    usePerHostIncludes: false,
    perHostIncludes: {},
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

  it('clears the shared includes and sends one entry per selected host', () => {
    const payload = agentOverridePayload(
      overrides({ usePerHostIncludes: true, perHostIncludes: { 1: '/home/keep' } }),
      [1, 2],
    )

    expect(payload.include_patterns_raw).toBe('')
    expect(payload.include_patterns_per_agent).toEqual([
      { agent_id: 1, raw_text: '/home/keep' },
      { agent_id: 2, raw_text: '' },
    ])
    expect(payload.exclude_patterns_per_agent).toBeUndefined()
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

  // "Use the shared values" turns every override off; without the flag that
  // would send nothing, and the server would keep the per-host rows it has.
  it('sends empty per-agent lists once the per-host values are to be cleared', () => {
    expect(agentOverridePayload(overrides({ clearPerHost: true }), [1, 2])).toEqual({
      exclude_patterns_per_agent: [],
      include_patterns_per_agent: [],
      file_change_patterns_per_agent: [],
      commands_per_agent: [],
    })
  })

  it('leaves the shared values alone when clearing the per-host ones', () => {
    const payload = agentOverridePayload(overrides({ clearPerHost: true }), [1])
    expect(payload.exclude_patterns_raw).toBeUndefined()
    expect(payload.pre_backup_commands).toBeUndefined()
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

describe('scheduleFormPayload filesystem snapshots', () => {
  it('sends the snapshot mode and the LVM snapshot size', () => {
    const payload = scheduleFormPayload(
      form({ fs_snapshot: 'best_effort', lvm_snapshot_percent: 25 }),
    )
    expect(payload).toMatchObject({ fs_snapshot: 'best_effort', lvm_snapshot_percent: 25 })
  })

  it('leaves out a hidden size the server would refuse, but sends a visible one', () => {
    for (const percent of [0, 101, 2.5]) {
      expect(
        scheduleFormPayload(form({ fs_snapshot: 'off', lvm_snapshot_percent: percent })),
      ).not.toHaveProperty('lvm_snapshot_percent')
      expect(
        scheduleFormPayload(form({ fs_snapshot: 'required', lvm_snapshot_percent: percent })),
      ).toMatchObject({ lvm_snapshot_percent: percent })
    }
  })

  it('starts a new schedule with snapshots off and the server default size', () => {
    expect(scheduleFormPayload(form())).toMatchObject({
      fs_snapshot: 'off',
      lvm_snapshot_percent: 10,
    })
  })
})

describe('ownPathsPayload', () => {
  const shared = { usePerHostPaths: false, perHostSources: {} }

  it('sends one shared list while paths are not split per host', () => {
    expect(ownPathsPayload(form({ backup_sources: '/home\n\n/etc' }), shared, [1, 2])).toEqual({
      backup_sources: ['/home', '/etc'],
    })
  })

  // Left in place, saved per-host paths would still win over the shared ones.
  it('deletes the per-host paths a schedule saved once the split is turned off', () => {
    const turnedOff = { ...shared, clearSavedPerHost: true }
    expect(ownPathsPayload(form({ backup_sources: '/srv' }), turnedOff, [1, 2])).toEqual({
      backup_sources: ['/srv'],
      backup_sources_per_agent: [],
    })
  })

  it('sends one list per host instead, leaving out a host with none', () => {
    const perHost = { usePerHostPaths: true, perHostSources: { 1: '/home', 2: '  ', 3: '/srv' } }
    expect(ownPathsPayload(form({ backup_sources: '/ignored' }), perHost, [1, 2])).toEqual({
      backup_sources: [],
      backup_sources_per_agent: [{ agent_id: 1, paths: ['/home'] }],
    })
  })
})

describe('sourcesProblem', () => {
  const shared = { usePerHostPaths: false, perHostSources: {} }

  it("takes each host's files as they are", () => {
    expect(sourcesProblem(form({ files_mode: 'host' }), shared, [1])).toBeNull()
  })

  it("needs every host to list paths when it takes each host's files", () => {
    const paths: Record<number, string[]> = { 1: [], 2: ['/etc'], 4: ['/srv'] }
    const lookup = (id: number) => paths[id]
    const bare = "paths on every host, or its own files: a host's Sources list none"
    expect(sourcesProblem(form({ files_mode: 'host' }), shared, [1, 2], lookup)).toBe(bare)
    // Not only the first host.
    expect(sourcesProblem(form({ files_mode: 'host' }), shared, [2, 1], lookup)).toBe(bare)
    expect(sourcesProblem(form({ files_mode: 'host' }), shared, [2, 4], lookup)).toBeNull()
    // A host it knows nothing about is left to the server.
    expect(sourcesProblem(form({ files_mode: 'host' }), shared, [3], lookup)).toBeNull()
  })

  it('needs a path for its own files, shared or per host', () => {
    expect(sourcesProblem(form({ files_mode: 'own' }), shared, [1])).toBe(
      'at least one backup path for its own files',
    )
    expect(
      sourcesProblem(form({ files_mode: 'own', backup_sources: '/srv' }), shared, [1]),
    ).toBeNull()
    const perHost = { usePerHostPaths: true, perHostSources: { 1: '/srv' } }
    expect(sourcesProblem(form({ files_mode: 'own' }), perHost, [1])).toBeNull()
  })

  it('needs virtual machines or databases when it reads no files', () => {
    expect(sourcesProblem(form({ files_mode: 'off' }), shared, [1])).toBe(
      'virtual machines or databases, since it reads no files',
    )
    expect(
      sourcesProblem(form({ files_mode: 'off', vm_snapshot_enabled: true }), shared, [1]),
    ).toBeNull()
    expect(
      sourcesProblem(form({ files_mode: 'off', database_backup_enabled: true }), shared, [1]),
    ).toBeNull()
  })
})
