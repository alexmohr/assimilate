// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'

import {
  actionForHealth,
  bytesToGb,
  computeSliceGeometry,
  gbToBytes,
  quotaCeiling,
  quotaHealth,
  type QuotaThresholds,
} from './quota'

describe('quotaHealth', () => {
  it('is unconfigured when there is no quota', () => {
    expect(quotaHealth(null, 1_000)).toBe('unconfigured')
    expect(quotaHealth(undefined, 1_000)).toBe('unconfigured')
  })

  it('is unconfigured when the quota is disabled, even with thresholds set', () => {
    const quota: QuotaThresholds = { warn_bytes: 100, critical_bytes: 200, enabled: false }
    expect(quotaHealth(quota, 500)).toBe('unconfigured')
  })

  it('is ok when usage is below both thresholds', () => {
    const quota: QuotaThresholds = { warn_bytes: 100, critical_bytes: 200, enabled: true }
    expect(quotaHealth(quota, 50)).toBe('ok')
  })

  it('is warning at or above the warn threshold', () => {
    const quota: QuotaThresholds = { warn_bytes: 100, critical_bytes: 200, enabled: true }
    expect(quotaHealth(quota, 100)).toBe('warning')
    expect(quotaHealth(quota, 150)).toBe('warning')
  })

  it('is critical at or above the critical threshold', () => {
    const quota: QuotaThresholds = { warn_bytes: 100, critical_bytes: 200, enabled: true }
    expect(quotaHealth(quota, 200)).toBe('critical')
    expect(quotaHealth(quota, 999)).toBe('critical')
  })

  it('handles a warn-only quota with no critical threshold', () => {
    const quota: QuotaThresholds = { warn_bytes: 100, critical_bytes: null, enabled: true }
    expect(quotaHealth(quota, 50)).toBe('ok')
    expect(quotaHealth(quota, 100)).toBe('warning')
  })
})

describe('quotaCeiling', () => {
  it('returns null when there is no quota', () => {
    expect(quotaCeiling(null)).toBeNull()
  })

  it('prefers critical_bytes over warn_bytes', () => {
    expect(quotaCeiling({ warn_bytes: 100, critical_bytes: 200, enabled: true })).toBe(200)
  })

  it('falls back to warn_bytes when critical_bytes is unset', () => {
    expect(quotaCeiling({ warn_bytes: 100, critical_bytes: null, enabled: true })).toBe(100)
  })

  it('is null when neither threshold is set', () => {
    expect(quotaCeiling({ warn_bytes: null, critical_bytes: null, enabled: true })).toBeNull()
  })
})

describe('actionForHealth', () => {
  it('returns null for ok and unconfigured', () => {
    expect(actionForHealth('ok', 'notify_only', 'block_backups')).toBeNull()
    expect(actionForHealth('unconfigured', 'notify_only', 'block_backups')).toBeNull()
  })

  it('returns the warn action for warning', () => {
    expect(actionForHealth('warning', 'notify_only', 'block_backups')).toBe('notify_only')
  })

  it('returns the critical action for critical', () => {
    expect(actionForHealth('critical', 'notify_only', 'block_backups')).toBe('block_backups')
  })
})

describe('bytesToGb / gbToBytes', () => {
  it('round-trips whole gigabytes', () => {
    expect(bytesToGb(gbToBytes(5))).toBe(5)
  })
})

describe('computeSliceGeometry', () => {
  const box = 1000

  it('has zero geometry when the box has no capacity', () => {
    const g = computeSliceGeometry({ offsetBytes: 0, usageBytes: 500, boxMaxBytes: 0 })
    expect(g.leftPercent).toBe(0)
    expect(g.fillWidthPercent).toBe(0)
  })

  it('positions a fill by offset and width by usage, as a percentage of the box', () => {
    const g = computeSliceGeometry({ offsetBytes: 200, usageBytes: 300, boxMaxBytes: box })
    expect(g.leftPercent).toBe(20)
    expect(g.fillWidthPercent).toBe(30)
  })

  it('clips the fill at the box edge when usage would run past it', () => {
    const g = computeSliceGeometry({ offsetBytes: 900, usageBytes: 300, boxMaxBytes: box })
    expect(g.leftPercent).toBe(90)
    expect(g.fillWidthPercent).toBe(10)
  })
})
