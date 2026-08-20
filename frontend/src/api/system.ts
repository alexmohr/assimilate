// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  ImportResultResponse,
  SettingsResponse,
  SshPublicKeyResponse,
  SystemResetResponse,
} from '../types/generated'

export interface VersionInfo {
  server_version: string
  server_git_sha: string
  build_timestamp: string
  agent_version: string | null
}

export interface DatabaseRelationSize {
  table_name: string
  table_bytes: number
  index_bytes: number
  toast_bytes: number
  total_bytes: number
}

export interface DatabaseStorageResponse {
  database_bytes: number
  other_bytes: number
  relations: DatabaseRelationSize[]
}

export interface UpdateSettingsRequest {
  retention_days: number
  report_retention_days: number
  failed_report_retention_days: number
  system_event_retention_days: number
  notification_delivery_retention_days: number
  timezone: string | undefined
  borg_query_timeout_secs: number
  session_idle_timeout_minutes: number
}

export async function getSshPublicKey(): Promise<SshPublicKeyResponse> {
  const response = await apiClient.get<SshPublicKeyResponse>('/system/ssh-public-key')
  return response.data
}

export async function regenerateSshKey(): Promise<SshPublicKeyResponse> {
  const response = await apiClient.post<SshPublicKeyResponse>('/system/ssh-regenerate-key')
  return response.data
}

export async function getSystemSettings(): Promise<SettingsResponse> {
  const response = await apiClient.get<SettingsResponse>('/system/settings')
  return response.data
}

export async function updateSystemSettings(data: UpdateSettingsRequest): Promise<SettingsResponse> {
  const response = await apiClient.put<SettingsResponse>('/system/settings', data)
  return response.data
}

export async function getSystemVersion(): Promise<VersionInfo> {
  const response = await apiClient.get<VersionInfo>('/system/version')
  return response.data
}

export async function getDatabaseStorage(): Promise<DatabaseStorageResponse> {
  const response = await apiClient.get<DatabaseStorageResponse>('/system/database-storage')
  return response.data
}

export async function resetSystem(): Promise<SystemResetResponse> {
  const response = await apiClient.post<SystemResetResponse>('/system/reset')
  return response.data
}

export async function exportConfig(): Promise<unknown> {
  const response = await apiClient.get<unknown>('/config/export')
  return response.data
}

export async function importConfig(payload: unknown): Promise<ImportResultResponse> {
  const response = await apiClient.post<ImportResultResponse>('/config/import', payload)
  return response.data
}

export async function getHealth(): Promise<void> {
  await apiClient.get('/health')
}
