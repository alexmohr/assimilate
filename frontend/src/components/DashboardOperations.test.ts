// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { renderWithPlugins } from '../test-utils'
import type { DashboardFinding, DashboardFindingKind } from '../types/dashboard'
import NeedsAttention from './NeedsAttention.vue'
import ProtectionCoverage from './ProtectionCoverage.vue'
import RepositoryCapacity from './RepositoryCapacity.vue'
import UpcomingWork from './UpcomingWork.vue'

vi.mock('../utils/format', () => ({
  formatBytes: (value: number): string => `${value}B`,
  relativeTime: (value: string): string => `relative:${value}`,
  formatDuration: (value: number): string => `${value}s`,
}))

vi.mock('../api/client', () => ({
  apiClient: { post: vi.fn().mockResolvedValue({}) },
}))

import { apiClient } from '../api/client'

const findingKinds: DashboardFindingKind[] = [
  'backup_failed',
  'backup_warning',
  'schedule_target_overdue',
  'schedule_target_never_succeeded',
  'host_offline_due_soon',
  'host_unassigned',
  'repository_unscheduled',
  'repository_quota_warning',
  'repository_quota_critical',
  'repository_import_failed',
]

function finding(kind: DashboardFindingKind, index: number): DashboardFinding {
  return {
    id: `finding-${index}`,
    kind,
    severity: index === 0 ? 'critical' : 'warning',
    status: kind === 'backup_failed' ? 'failed' : 'warning',
    hostname: `host-${index}`,
    schedule_id: index + 1,
    schedule_name: `schedule-${index}`,
    repo_id: index + 10,
    repo_name: `repo-${index}`,
    reason: `reason-${kind}`,
    occurred_at: '2026-06-01T00:00:00Z',
    deadline: null,
    destination: { kind: 'schedule', schedule_id: index + 1 },
  }
}

describe('dashboard operational components', () => {
  beforeEach(() => {
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.post).mockResolvedValue({})
  })

  it('renders every finding kind and direct schedule links', () => {
    const findings = findingKinds.map(finding)
    const wrapper = renderWithPlugins(NeedsAttention, { props: { findings } })

    findingKinds.forEach((kind) => expect(wrapper.text()).toContain(`reason-${kind}`))
    expect(wrapper.findAll('.finding-row')).toHaveLength(findingKinds.length)
    expect(wrapper.find('.finding-row .finding-action').attributes('href')).toBe('/schedules/1')
  })

  it('renders the no-problems empty state', () => {
    const wrapper = renderWithPlugins(NeedsAttention, { props: { findings: [] } })
    expect(wrapper.text()).toContain('No active problems')
  })

  it('dismiss button calls the api and emits dismissed', async () => {
    const f = finding('backup_failed', 0)
    const wrapper = renderWithPlugins(NeedsAttention, { props: { findings: [f] } })

    await wrapper.find('.dismiss-btn').trigger('click')
    await nextTick()

    expect(apiClient.post).toHaveBeenCalledWith('/stats/findings/finding-0/dismiss')
    expect(wrapper.emitted('dismissed')).toBeTruthy()
  })

  it('routes backup_failed activity destination to filtered activity log', () => {
    const f: DashboardFinding = {
      ...finding('backup_failed', 0),
      schedule_id: 7,
      destination: { kind: 'activity', report_id: 99 },
    }
    const wrapper = renderWithPlugins(NeedsAttention, { props: { findings: [f] } })
    const href = wrapper.find('.finding-action').attributes('href')
    expect(href).toContain('/activity')
    expect(href).toContain('status=failed')
    expect(href).toContain('schedule_id=7')
    expect(href).toContain('category=backup')
  })

  it('routes backup_warning activity destination to filtered activity log', () => {
    const f: DashboardFinding = {
      ...finding('backup_warning', 1),
      schedule_id: null,
      destination: { kind: 'activity', report_id: 88 },
    }
    const wrapper = renderWithPlugins(NeedsAttention, { props: { findings: [f] } })
    const href = wrapper.find('.finding-action').attributes('href')
    expect(href).toContain('/activity')
    expect(href).toContain('status=warning')
    expect(href).not.toContain('schedule_id')
  })

  it('shows precise protection counts and host navigation', () => {
    const wrapper = renderWithPlugins(ProtectionCoverage, {
      props: {
        protection: {
          protected_hosts: 2,
          eligible_hosts: 3,
          protected_agent_links: [
            { agent_id: 5, hostname: 'protected-host' },
            { agent_id: 6, hostname: 'protected-host-2' },
          ],
          unassigned_agents: [{ agent_id: 7, hostname: 'unassigned-host' }],
          never_succeeded_targets: 1,
          never_succeeded_agents: [{ agent_id: 9, hostname: 'never-succeeded-host' }],
          disabled_only_agents: [{ agent_id: 8, hostname: 'disabled-host' }],
        },
      },
    })

    expect(wrapper.text()).toContain('2/3')
    expect(wrapper.text()).toContain('unassigned-host')
    expect(wrapper.find('.host-links a').attributes('href')).toBe('/agents/unassigned-host')
    expect(wrapper.find('.coverage-score').attributes('href')).toBe('/agents?coverage=protected')
    expect(wrapper.findAll('.coverage-facts a').map((link) => link.attributes('href'))).toEqual([
      '/agents?coverage=unassigned',
      '/agents?coverage=never-succeeded',
      '/agents?coverage=disabled-only',
    ])
  })

  it('groups running and upcoming work by operation and schedule', () => {
    const wrapper = renderWithPlugins(UpcomingWork, {
      props: {
        operations: [
          {
            report_id: 12,
            status: 'running',
            hostname: 'db-01',
            schedule_id: 2,
            schedule_name: 'Database hourly',
            repo_id: 3,
            repo_name: 'database',
            started_at: '2026-06-01T00:00:00Z',
            destination: { kind: 'activity', report_id: 12 },
          },
        ],
        schedules: [
          {
            schedule_id: 4,
            schedule_name: 'Fleet daily',
            repo_id: 5,
            repo_name: 'daily',
            next_run_at: '2026-06-02T00:00:00Z',
            target_count: 8,
            offline_target_count: 2,
          },
        ],
      },
    })

    expect(wrapper.text()).toContain('Running relative:2026-06-01T00:00:00Z')
    expect(wrapper.text()).toMatch(/8 targets\s+\u00B7 2 offline/)
    expect(wrapper.findAll('.work-row')).toHaveLength(2)
  })

  it('renders quota states and insufficient-history fallback', () => {
    const wrapper = renderWithPlugins(RepositoryCapacity, {
      props: {
        repositories: [
          {
            repo_id: 9,
            repo_name: 'critical-repo',
            deduplicated_size: 900,
            quota_bytes: 1000,
            quota_utilization_percent: 90,
            quota_status: 'critical',
            storage_change_bytes: null,
            threshold_estimate: null,
          },
        ],
      },
    })

    expect(wrapper.text()).toContain('900B deduplicated')
    expect(wrapper.text()).toContain('90% of 1000B')
    expect(wrapper.text()).toContain('Insufficient history')
    expect(wrapper.find('.capacity-row').attributes('href')).toBe('/repos/9')
  })
})
