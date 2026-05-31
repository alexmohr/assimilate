// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'

interface HealthEntry {
  hostname: string
  target_name: string
  last_status: string | null
  last_backup_at: string | null
  is_overdue: boolean
  last_error_message: string | null
  cron_expression: string | null
  schedule_enabled: boolean | null
}

interface RepoRow {
  id: number
  name: string
  repo_path: string
  enabled: boolean
}

interface ScheduleRow {
  id: number
  repo_id: number
  enabled: boolean
}

function buildHealthByRepo(health: HealthEntry[]): Map<string, HealthEntry[]> {
  const m = new Map<string, HealthEntry[]>()
  health.forEach((h) => {
    const entries = m.get(h.target_name) ?? []
    entries.push(h)
    m.set(h.target_name, entries)
  })
  return m
}

function pickHealthForSchedule(
  schedule: ScheduleRow,
  repoMap: Map<number, RepoRow>,
  healthByRepo: Map<string, HealthEntry[]>,
): HealthEntry | null {
  const repo = repoMap.get(schedule.repo_id) ?? null
  const entries = repo ? (healthByRepo.get(repo.name) ?? []) : []
  return (
    entries.find((h) => h.is_overdue) ??
    entries.find((h) => h.last_status === 'failed') ??
    entries[0] ??
    null
  )
}

function makeHealth(overrides: Partial<HealthEntry> = {}): HealthEntry {
  return {
    hostname: 'host-01',
    target_name: 'my-repo',
    last_status: 'success',
    last_backup_at: '2026-01-01T00:00:00Z',
    is_overdue: false,
    last_error_message: null,
    cron_expression: '0 2 * * *',
    schedule_enabled: true,
    ...overrides,
  }
}

describe('SchedulesView health enrichment', () => {
  const repo: RepoRow = { id: 1, name: 'my-repo', repo_path: '/repo', enabled: true }
  const repoMap = new Map<number, RepoRow>([[1, repo]])

  describe('buildHealthByRepo', () => {
    it('groups entries by target_name', () => {
      const entries = [
        makeHealth({ hostname: 'host-01', target_name: 'repo-a' }),
        makeHealth({ hostname: 'host-02', target_name: 'repo-a' }),
        makeHealth({ hostname: 'host-01', target_name: 'repo-b' }),
      ]
      const map = buildHealthByRepo(entries)
      expect(map.get('repo-a')).toHaveLength(2)
      expect(map.get('repo-b')).toHaveLength(1)
      expect(map.has('repo-c')).toBe(false)
    })

    it('returns empty map for empty input', () => {
      expect(buildHealthByRepo([])).toEqual(new Map())
    })
  })

  describe('pickHealthForSchedule', () => {
    it('returns null when repo not found', () => {
      const schedule: ScheduleRow = { id: 1, repo_id: 999, enabled: true }
      const result = pickHealthForSchedule(schedule, repoMap, new Map())
      expect(result).toBeNull()
    })

    it('returns null when no health entries exist for repo', () => {
      const schedule: ScheduleRow = { id: 1, repo_id: 1, enabled: true }
      const result = pickHealthForSchedule(schedule, repoMap, new Map())
      expect(result).toBeNull()
    })

    it('returns first entry when all are successful', () => {
      const entries = [makeHealth({ hostname: 'host-01' }), makeHealth({ hostname: 'host-02' })]
      const healthByRepo = buildHealthByRepo(entries)
      const schedule: ScheduleRow = { id: 1, repo_id: 1, enabled: true }
      const result = pickHealthForSchedule(schedule, repoMap, healthByRepo)
      expect(result?.hostname).toBe('host-01')
      expect(result?.is_overdue).toBe(false)
    })

    it('prioritizes overdue entry over success', () => {
      const entries = [
        makeHealth({ hostname: 'host-01', is_overdue: false }),
        makeHealth({ hostname: 'host-02', is_overdue: true }),
      ]
      const healthByRepo = buildHealthByRepo(entries)
      const schedule: ScheduleRow = { id: 1, repo_id: 1, enabled: true }
      const result = pickHealthForSchedule(schedule, repoMap, healthByRepo)
      expect(result?.hostname).toBe('host-02')
      expect(result?.is_overdue).toBe(true)
    })

    it('prioritizes overdue over failed', () => {
      const entries = [
        makeHealth({ hostname: 'host-01', last_status: 'failed', is_overdue: false }),
        makeHealth({ hostname: 'host-02', is_overdue: true }),
      ]
      const healthByRepo = buildHealthByRepo(entries)
      const schedule: ScheduleRow = { id: 1, repo_id: 1, enabled: true }
      const result = pickHealthForSchedule(schedule, repoMap, healthByRepo)
      expect(result?.hostname).toBe('host-02')
    })

    it('falls back to failed when none are overdue', () => {
      const entries = [
        makeHealth({ hostname: 'host-01', last_status: 'success' }),
        makeHealth({ hostname: 'host-02', last_status: 'failed' }),
      ]
      const healthByRepo = buildHealthByRepo(entries)
      const schedule: ScheduleRow = { id: 1, repo_id: 1, enabled: true }
      const result = pickHealthForSchedule(schedule, repoMap, healthByRepo)
      expect(result?.hostname).toBe('host-02')
      expect(result?.last_status).toBe('failed')
    })
  })

  describe('overdue filter integration', () => {
    it('filter matches schedules with overdue health', () => {
      const entries = [makeHealth({ target_name: 'my-repo', is_overdue: true })]
      const healthByRepo = buildHealthByRepo(entries)
      const schedule: ScheduleRow = { id: 1, repo_id: 1, enabled: true }
      const health = pickHealthForSchedule(schedule, repoMap, healthByRepo)
      expect(health?.is_overdue).toBe(true)
    })

    it('filter does NOT match schedules without overdue health', () => {
      const entries = [makeHealth({ target_name: 'my-repo', is_overdue: false })]
      const healthByRepo = buildHealthByRepo(entries)
      const schedule: ScheduleRow = { id: 1, repo_id: 1, enabled: true }
      const health = pickHealthForSchedule(schedule, repoMap, healthByRepo)
      expect(health?.is_overdue).toBe(false)
    })

    it('filter does NOT match schedules with no health data', () => {
      const schedule: ScheduleRow = { id: 1, repo_id: 1, enabled: true }
      const health = pickHealthForSchedule(schedule, repoMap, new Map())
      expect(health?.is_overdue).toBeUndefined()
    })
  })
})
