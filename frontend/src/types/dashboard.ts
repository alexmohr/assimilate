// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

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

export interface DashboardFinding {
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

export interface DashboardHostLink {
  client_id: number
  hostname: string
}

export interface DashboardOperation {
  report_id: number
  status: 'running'
  hostname: string
  schedule_id: number
  schedule_name: string
  repo_id: number
  repo_name: string
  started_at: string
  destination: DashboardDestination
}

export interface DashboardUpcomingSchedule {
  schedule_id: number
  schedule_name: string
  repo_id: number
  repo_name: string
  next_run_at: string
  target_count: number
  offline_target_count: number
}

export type DashboardQuotaStatus = 'unconfigured' | 'healthy' | 'warning' | 'critical'

export interface DashboardRepositoryCapacity {
  repo_id: number
  repo_name: string
  deduplicated_size: number
  quota_bytes: number | null
  quota_utilization_percent: number | null
  quota_status: DashboardQuotaStatus
  storage_change_bytes: number | null
  threshold_estimate: string | null
}

export interface DashboardOverview {
  summary: {
    protected_hosts: number
    eligible_hosts: number
    needs_attention: number
    running_operations: number
    total_storage_bytes: number
  }
  findings: DashboardFinding[]
  protection: {
    protected_hosts: number
    eligible_hosts: number
    unassigned_hosts: DashboardHostLink[]
    never_succeeded_targets: number
    disabled_only_hosts: DashboardHostLink[]
  }
  running_operations: DashboardOperation[]
  upcoming_schedules: DashboardUpcomingSchedule[]
  repository_capacity: DashboardRepositoryCapacity[]
}
