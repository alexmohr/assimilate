// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  CreateTunnelRequest,
  SshTunnel,
  TunnelWithStatus,
  UpdateTunnelRequest,
} from '../types/tunnel'

export async function listTunnels(): Promise<TunnelWithStatus[]> {
  const response = await apiClient.get<TunnelWithStatus[]>('/tunnels')
  return response.data
}

export async function getTunnel(id: number): Promise<TunnelWithStatus> {
  const response = await apiClient.get<TunnelWithStatus>(`/tunnels/${id}`)
  return response.data
}

export async function createTunnel(data: CreateTunnelRequest): Promise<SshTunnel> {
  const response = await apiClient.post<SshTunnel>('/tunnels', data)
  return response.data
}

export async function updateTunnel(id: number, data: UpdateTunnelRequest): Promise<SshTunnel> {
  const response = await apiClient.put<SshTunnel>(`/tunnels/${id}`, data)
  return response.data
}

export async function deleteTunnel(id: number): Promise<void> {
  await apiClient.delete(`/tunnels/${id}`)
}

export async function enableTunnel(id: number): Promise<void> {
  await apiClient.post(`/tunnels/${id}/enable`)
}

export async function disableTunnel(id: number): Promise<void> {
  await apiClient.post(`/tunnels/${id}/disable`)
}

export async function reconnectTunnel(id: number): Promise<TunnelWithStatus> {
  const response = await apiClient.post<TunnelWithStatus>(`/tunnels/${id}/reconnect`)
  return response.data
}

export async function getClientTunnel(hostname: string): Promise<TunnelWithStatus | null> {
  try {
    const response = await apiClient.get<TunnelWithStatus>(`/clients/${hostname}/tunnel`)
    return response.data
  } catch {
    return null
  }
}
