// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import type { Router } from 'vue-router'
import type { ReportRow } from '../types/report'
import { openReportArchive } from './reportNavigation'

function report(overrides: Partial<ReportRow> = {}): ReportRow {
  return { repo_id: 7, archive_name: null, ...overrides } as unknown as ReportRow
}

describe('openReportArchive', () => {
  it('navigates to the report repo Backups tab with no archive param when none was recorded', () => {
    const push = vi.fn()
    openReportArchive({ push } as unknown as Router, report())

    expect(push).toHaveBeenCalledWith({ path: '/repos/7', query: { tab: 'archives' } })
  })

  it('pre-selects the archive the report wrote', () => {
    const push = vi.fn()
    openReportArchive({ push } as unknown as Router, report({ archive_name: 'web-01-2026-01-01' }))

    expect(push).toHaveBeenCalledWith({
      path: '/repos/7',
      query: { tab: 'archives', archive: 'web-01-2026-01-01' },
    })
  })
})
