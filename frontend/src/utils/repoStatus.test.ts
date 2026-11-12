// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { repoFixture } from '../test-utils/repoFixtures'
import { isRepoProblem, repoQuotaUse, repoRowStatus } from './repoStatus'
import type { RepoWithStats } from '../types/repo'

type Quota = NonNullable<RepoWithStats['quota']>

function quota(overrides: Partial<Quota> = {}): Quota {
  return {
    warn_bytes: 500,
    critical_bytes: 1000,
    warn_action: 'notify_only',
    critical_action: 'block_backups',
    enabled: true,
    ...overrides,
  }
}

function repo(overrides: Partial<RepoWithStats> = {}): RepoWithStats {
  return repoFixture({ unmatched_count: 0, quota: null, ...overrides })
}

describe('repoRowStatus', () => {
  it('reads healthy when nothing needs attention', () => {
    const status = repoRowStatus(repo())
    expect(status).toMatchObject({ kind: 'healthy', label: 'Healthy', tone: 'success' })
    expect(isRepoProblem(status)).toBe(false)
  })

  it('flags a failed import with the error behind it', () => {
    const status = repoRowStatus(repo({ import_error: 'boom', importing: true }))
    expect(status).toMatchObject({
      kind: 'import_failed',
      label: 'Import failed',
      tone: 'danger',
      detail: 'boom',
    })
    expect(isRepoProblem(status)).toBe(true)
  })

  it('shows an open-ended import before the total is known', () => {
    expect(repoRowStatus(repo({ importing: true, import_total: 0 })).label).toBe('Importing…')
  })

  it('shows import progress with the phase from the status message', () => {
    const status = repoRowStatus(
      repo({
        importing: true,
        import_progress: 1,
        import_total: 4,
        import_status_message: 'Indexing archive 1 of 4',
      }),
    )
    expect(status).toMatchObject({
      kind: 'importing',
      label: 'Indexing 1/4',
      tone: 'warning',
      detail: 'Indexing archive 1 of 4',
    })
  })

  it('reads disabled for a switched-off repository, ahead of its quota', () => {
    const status = repoRowStatus(
      repo({ enabled: false, quota: quota(), total_deduplicated_size: 2000 }),
    )
    expect(status).toMatchObject({ kind: 'disabled', label: 'Disabled', tone: 'neutral' })
    expect(isRepoProblem(status)).toBe(false)
  })

  it('reads over quota at the critical threshold, naming the action', () => {
    const status = repoRowStatus(repo({ quota: quota(), total_deduplicated_size: 1000 }))
    expect(status).toMatchObject({
      kind: 'over_quota',
      label: 'Over quota',
      tone: 'danger',
      detail: 'Block backups',
    })
  })

  it('reads near quota at the warn threshold', () => {
    const status = repoRowStatus(repo({ quota: quota(), total_deduplicated_size: 600 }))
    expect(status).toMatchObject({ kind: 'near_quota', label: 'Near quota', tone: 'warning' })
  })

  it('ignores a disabled quota', () => {
    const status = repoRowStatus(
      repo({ quota: quota({ enabled: false }), total_deduplicated_size: 2000 }),
    )
    expect(status.kind).toBe('healthy')
  })

  it('counts unmatched archives when the quota is fine, without calling them a problem', () => {
    const status = repoRowStatus(repo({ unmatched_count: 2 }))
    expect(status).toMatchObject({
      kind: 'unmatched',
      label: '2 unmatched',
      tone: 'success',
      detail: 'Imported archives no schedule claims yet',
    })
    // Unmatched archives are old data to sort out, not a backup at risk.
    expect(isRepoProblem(status)).toBe(false)
  })

  it('puts a near-quota warning ahead of unmatched archives', () => {
    const status = repoRowStatus(
      repo({ unmatched_count: 2, quota: quota(), total_deduplicated_size: 600 }),
    )
    expect(status.kind).toBe('near_quota')
    expect(isRepoProblem(status)).toBe(true)
  })

  it('puts a quota breach ahead of unmatched archives', () => {
    const status = repoRowStatus(
      repo({ unmatched_count: 2, quota: quota(), total_deduplicated_size: 1000 }),
    )
    expect(status.kind).toBe('over_quota')
  })
})

describe('repoQuotaUse', () => {
  it('is null without a quota, with a disabled one, or with no threshold', () => {
    expect(repoQuotaUse(repo())).toBeNull()
    expect(repoQuotaUse(repo({ quota: quota({ enabled: false }) }))).toBeNull()
    expect(
      repoQuotaUse(repo({ quota: quota({ warn_bytes: null, critical_bytes: null }) })),
    ).toBeNull()
    expect(repoQuotaUse(repo({ quota: quota({ warn_bytes: 0, critical_bytes: 0 }) }))).toBeNull()
  })

  it('measures against the critical threshold', () => {
    expect(repoQuotaUse(repo({ quota: quota(), total_deduplicated_size: 250 }))).toEqual({
      ceilingBytes: 1000,
      fillPercent: 25,
    })
  })

  it('falls back to the warn threshold when critical is unset', () => {
    expect(
      repoQuotaUse(repo({ quota: quota({ critical_bytes: null }), total_deduplicated_size: 250 })),
    ).toEqual({ ceilingBytes: 500, fillPercent: 50 })
  })

  it('clips the fill at 100% past the ceiling', () => {
    expect(repoQuotaUse(repo({ quota: quota(), total_deduplicated_size: 5000 }))?.fillPercent).toBe(
      100,
    )
  })
})
