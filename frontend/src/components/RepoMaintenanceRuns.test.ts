// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import RepoMaintenanceRuns from './RepoMaintenanceRuns.vue'
import type { MaintenanceRunResponse, PendingDeletionResponse } from '../types/generated'

function run(overrides: Partial<MaintenanceRunResponse>): MaintenanceRunResponse {
  return {
    id: 1,
    started_at: '2026-10-26T04:30:00Z',
    finished_at: '2026-10-26T04:44:00Z',
    outcome: 'ok',
    archives_removed: 21,
    bytes_freed: 1024,
    detail: null,
    ...overrides,
  }
}

function mount(pending: PendingDeletionResponse[] = [], runs: MaintenanceRunResponse[] = []) {
  return renderWithPlugins(RepoMaintenanceRuns, { props: { pending, runs, busy: false } })
}

describe('RepoMaintenanceRuns', () => {
  it('says when nothing is pending or has run', () => {
    const text = mount().text()
    expect(text).toContain('Nothing is waiting.')
    expect(text).toContain('No maintenance run yet.')
    expect(mount().find('table').exists()).toBe(false)
  })

  it('lists queued deletions with who asked and offers Keep', async () => {
    const wrapper = mount([
      {
        archive_name: 'photos-import-test',
        reason: 'manual',
        requested_by: 'admin',
        requested_at: '2026-10-25T10:00:00Z',
      },
      {
        archive_name: 'old',
        reason: 'manual',
        requested_by: null,
        requested_at: '2026-10-25T10:00:00Z',
      },
    ])
    const rows = wrapper.findAll('tbody tr')
    expect(rows[0]!.text()).toContain('photos-import-test')
    expect(rows[0]!.text()).toContain('admin')
    expect(rows[1]!.text()).toContain('Unknown')

    await rows[0]!.find('button').trigger('click')
    expect(wrapper.emitted('keep')).toEqual([['photos-import-test']])
  })

  it.each([
    ['ok', 'OK', 'badge--success'],
    ['stopped', 'Stopped', 'badge--warning'],
    ['check_failed', 'Check failed', 'badge--danger'],
    ['failed', 'Failed', 'badge--danger'],
  ])('badges a %s run', (outcome, label, tone) => {
    const badge = mount([], [run({ outcome })]).find('.badge')
    expect(badge.text()).toBe(label)
    expect(badge.classes()).toContain(tone)
  })

  it('shows the figures, the detail and the duration of a run', () => {
    const row = mount([], [run({ outcome: 'stopped', detail: '2 archives vanished' })]).find(
      'tbody tr',
    )
    expect(row.text()).toContain('2 archives vanished')
    expect(row.text()).toContain('21')
    expect(row.text()).toContain('1.0 KB')
    expect(row.text()).toContain('14m 0s')
  })

  it('shows a run that has not finished as running', () => {
    expect(
      mount([], [run({ finished_at: null })])
        .find('tbody tr')
        .text(),
    ).toContain('Running')
  })
})
