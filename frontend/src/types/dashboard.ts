// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  DashboardAgentLinkResponse,
  DashboardOperationResponse,
  DashboardUpcomingScheduleResponse,
  DashboardRepositoryCapacityResponse,
  DashboardSummaryCountersResponse,
  DashboardProtectionCoverageResponse,
} from './generated'

export type DashboardStatus =
  | 'healthy'
  | 'warning'
  | 'failed'
  | 'overdue'
  | 'never_succeeded'
  | 'running'
  | 'disabled'
  | 'offline_due_soon'

export type DashboardSeverity = 'critical' | 'warning' | 'info'

export type DashboardFindingKind =
  | 'backup_failed'
  | 'backup_warning'
  | 'schedule_target_overdue'
  | 'schedule_target_never_succeeded'
  | 'host_offline_due_soon'
  | 'host_unassigned'
  | 'repository_unscheduled'
  | 'repository_quota_warning'
  | 'repository_quota_critical'
  | 'repository_import_failed'

export type DashboardDestination =
  | { kind: 'host'; hostname: string }
  | { kind: 'schedule'; schedule_id: number }
  | { kind: 'repository'; repo_id: number }
  | { kind: 'activity'; report_id: number }

export type DashboardFinding = {
  id: string
  kind: DashboardFindingKind
  severity: DashboardSeverity
  status: DashboardStatus
  hostname: string | null
  schedule_id: number | null
  schedule_name: string | null
  repo_id: number | null
  repo_name: string | null
  reason: string
  occurred_at: string | null
  deadline: string | null
  destination: DashboardDestination
}

export type DashboardHostLink = Omit<DashboardAgentLinkResponse, 'agent_id'> & { agent_id: number }

export type DashboardOperation = Omit<
  DashboardOperationResponse,
  'report_id' | 'schedule_id' | 'repo_id'
> & {
  report_id: number
  schedule_id: number
  repo_id: number
  destination: DashboardDestination
}

export type DashboardUpcomingSchedule = Omit<
  DashboardUpcomingScheduleResponse,
  'schedule_id' | 'repo_id' | 'target_count'
> & {
  schedule_id: number
  repo_id: number
  target_count: number
}

export type DashboardQuotaStatus = 'unconfigured' | 'healthy' | 'warning' | 'critical'

export type DashboardRepositoryCapacity = Omit<
  DashboardRepositoryCapacityResponse,
  'repo_id' | 'deduplicated_size' | 'quota_bytes' | 'storage_change_bytes'
> & {
  repo_id: number
  deduplicated_size: number
  quota_bytes: number | null
  storage_change_bytes: number | null
  quota_status: DashboardQuotaStatus
}

export type DashboardSummaryCounters = Omit<
  DashboardSummaryCountersResponse,
  'protected_hosts' | 'eligible_hosts' | 'total_storage_bytes'
> & {
  protected_hosts: number
  eligible_hosts: number
  total_storage_bytes: number
}

export type DashboardProtectionCoverage = Omit<
  DashboardProtectionCoverageResponse,
  | 'protected_hosts'
  | 'eligible_hosts'
  | 'never_succeeded_targets'
  | 'protected_agent_links'
  | 'unassigned_agents'
  | 'never_succeeded_agents'
  | 'disabled_only_agents'
> & {
  protected_hosts: number
  eligible_hosts: number
  never_succeeded_targets: number
  protected_agent_links: DashboardHostLink[]
  unassigned_agents: DashboardHostLink[]
  never_succeeded_agents: DashboardHostLink[]
  disabled_only_agents: DashboardHostLink[]
}

export type DashboardOverview = {
  summary: DashboardSummaryCounters
  findings: DashboardFinding[]
  protection: DashboardProtectionCoverage
  running_operations: DashboardOperation[]
  upcoming_schedules: DashboardUpcomingSchedule[]
  repository_capacity: DashboardRepositoryCapacity[]
}
