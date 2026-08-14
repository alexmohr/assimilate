// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { QuotaAction } from '../types/generated'

const BYTES_PER_GB = 1073741824

const QUOTA_ACTION_LABELS: Record<QuotaAction, string> = {
  notify_only: 'Notify only',
  block_backups: 'Block backups',
  disable_schedule: 'Disable schedule',
}

export function actionLabel(action: QuotaAction): string {
  return QUOTA_ACTION_LABELS[action]
}

export function bytesToGb(bytes: number): number {
  return Math.round((bytes / BYTES_PER_GB) * 100) / 100
}

/** Entering 0 (or a negative value) persists a 0-byte threshold, it does not clear it. */
export function gbToBytes(gb: number): number {
  return Math.round(gb * BYTES_PER_GB)
}

/** Minimal shape shared by repo- and server-level quota thresholds. */
export interface QuotaThresholds {
  warn_bytes: number | null
  critical_bytes: number | null
  enabled: boolean
}

export type QuotaHealth = 'unconfigured' | 'ok' | 'warning' | 'critical'

/** Mirrors the server's `repository_quota_status`/`evaluate_thresholds` logic. */
export function quotaHealth(
  quota: QuotaThresholds | null | undefined,
  usageBytes: number,
): QuotaHealth {
  if (!quota || !quota.enabled) return 'unconfigured'
  if (quota.critical_bytes !== null && usageBytes >= quota.critical_bytes) return 'critical'
  if (quota.warn_bytes !== null && usageBytes >= quota.warn_bytes) return 'warning'
  return 'ok'
}

/** The effective ceiling used for a percentage/progress display: critical, falling back to warn. */
export function quotaCeiling(quota: QuotaThresholds | null | undefined): number | null {
  if (!quota) return null
  return quota.critical_bytes ?? quota.warn_bytes
}

/** The action that fires for the given health, or null when usage is within bounds. */
export function actionForHealth(
  health: QuotaHealth,
  warnAction: QuotaAction,
  criticalAction: QuotaAction,
): QuotaAction | null {
  if (health === 'critical') return criticalAction
  if (health === 'warning') return warnAction
  return null
}

/** Geometry for drawing one repo's slice inside a host-quota pool bar. */
export interface SliceGeometry {
  /** Left edge of this repo's slice, as a percentage of the box (0-100). */
  leftPercent: number
  /** Width of this repo's usage fill, as a percentage of the box (0-100), clipped to the box edge. */
  fillWidthPercent: number
  /** Whether this repo has its own quota configured (warn or critical > 0, and enabled). */
  hasOwnQuota: boolean
  /** Width of the own-quota bracket, as a percentage of the box, clipped to the box edge. */
  bracketWidthPercent: number
  /** True when the repo's own ceiling would extend past the box's total capacity. */
  bracketOvercommit: boolean
  /** Own warn-threshold tick position, as a percentage of the box, or null when not shown. */
  tickPercent: number | null
  /** True when usage has exceeded the repo's own ceiling. */
  pastOwnLimit: boolean
  /** Bytes of usage past the own ceiling, when pastOwnLimit is true, else 0. */
  overOwnBytes: number
  /** Bytes remaining under the own ceiling, when not pastOwnLimit and an own quota exists. */
  headroomBytes: number | null
  /** This repo's own quota health, used to color the chip. 'unconfigured' when hasOwnQuota is false. */
  ownHealth: QuotaHealth
}

/**
 * Computes the geometry for one repo's slice inside a host-quota pool bar, where every repo in
 * the group shares one scale (the host's quota ceiling) and is positioned by the cumulative bytes
 * of the repos before it.
 */
export function computeSliceGeometry(params: {
  offsetBytes: number
  usageBytes: number
  boxMaxBytes: number
  quota: QuotaThresholds | null | undefined
}): SliceGeometry {
  const { offsetBytes, usageBytes, boxMaxBytes, quota } = params
  const safeBox = boxMaxBytes > 0 ? boxMaxBytes : 0
  const pct = (bytes: number): number =>
    safeBox > 0 ? Math.max(0, Math.min(100, (bytes / safeBox) * 100)) : 0

  const ownHealth = quotaHealth(quota, usageBytes)
  const ownCeiling = quota?.enabled ? quotaCeiling(quota) : null
  const hasOwnQuota = ownCeiling !== null && ownCeiling > 0

  const leftPercent = pct(offsetBytes)
  const fillWidthPercent = Math.max(0, pct(offsetBytes + usageBytes) - leftPercent)

  const bracketWidthPercent = hasOwnQuota
    ? Math.max(0, pct(offsetBytes + (ownCeiling as number)) - leftPercent)
    : 0
  const bracketOvercommit =
    hasOwnQuota && safeBox > 0 && offsetBytes + (ownCeiling as number) > safeBox

  const pastOwnLimit = hasOwnQuota && usageBytes > (ownCeiling as number)
  const overOwnBytes = pastOwnLimit ? usageBytes - (ownCeiling as number) : 0
  const headroomBytes = hasOwnQuota && !pastOwnLimit ? (ownCeiling as number) - usageBytes : null

  const warnBytes = quota?.warn_bytes ?? null
  const tickPercent =
    hasOwnQuota && warnBytes !== null && warnBytes > 0 && warnBytes < (ownCeiling as number)
      ? pct(offsetBytes + warnBytes)
      : null

  return {
    leftPercent,
    fillWidthPercent,
    hasOwnQuota,
    bracketWidthPercent,
    bracketOvercommit,
    tickPercent,
    pastOwnLimit,
    overOwnBytes,
    headroomBytes,
    ownHealth,
  }
}
