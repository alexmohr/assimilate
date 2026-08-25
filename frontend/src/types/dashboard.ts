// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  DashboardAgentLinkResponse,
  DashboardDestinationResponse,
  DashboardFindingResponse,
  DashboardOperationResponse,
  DashboardUpcomingScheduleResponse,
  DashboardRepositoryCapacityResponse,
  DashboardSummaryCountersResponse,
  DashboardProtectionCoverageResponse,
} from './generated'

export type DashboardDestination = DashboardDestinationResponse

export type DashboardFinding = DashboardFindingResponse

export type DashboardHostLink = DashboardAgentLinkResponse

export type DashboardOperation = DashboardOperationResponse

export type DashboardUpcomingSchedule = DashboardUpcomingScheduleResponse

// Mirrors `DashboardQuotaStatus` in crates/server/src/api/stats.rs, which isn't part of the
// ts-rs export surface (only crates/shared exports bindings) and isn't Serialize/TS-derived.
export type DashboardQuotaStatus = 'unconfigured' | 'healthy' | 'warning' | 'critical'

export type DashboardRepositoryCapacity = Omit<
  DashboardRepositoryCapacityResponse,
  'quota_bytes' | 'storage_change_bytes' | 'quota_status'
> & {
  quota_bytes: number | null
  storage_change_bytes: number | null
  quota_status: DashboardQuotaStatus
}

export type DashboardSummaryCounters = DashboardSummaryCountersResponse

export type DashboardProtectionCoverage = DashboardProtectionCoverageResponse

export type DashboardOverview = {
  summary: DashboardSummaryCounters
  findings: DashboardFinding[]
  protection: DashboardProtectionCoverage
  running_operations: DashboardOperation[]
  upcoming_schedules: DashboardUpcomingSchedule[]
  repository_capacity: DashboardRepositoryCapacity[]
}
