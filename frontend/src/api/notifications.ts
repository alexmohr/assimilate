// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  CreateChannelRequest,
  CreateRuleRequest,
  NotificationChannel,
  NotificationDelivery,
  NotificationRule,
  PushSubscriptionInfo,
  UpdateChannelRequest,
} from '../types/notifications'

export async function listChannels(): Promise<NotificationChannel[]> {
  const response = await apiClient.get<NotificationChannel[]>('/notifications/channels')
  return response.data
}

export async function createChannel(data: CreateChannelRequest): Promise<NotificationChannel> {
  const response = await apiClient.post<NotificationChannel>('/notifications/channels', data)
  return response.data
}

export async function updateChannel(
  id: number,
  data: UpdateChannelRequest,
): Promise<NotificationChannel> {
  const response = await apiClient.put<NotificationChannel>(`/notifications/channels/${id}`, data)
  return response.data
}

export async function deleteChannel(id: number): Promise<void> {
  await apiClient.delete(`/notifications/channels/${id}`)
}

export async function testChannel(id: number): Promise<void> {
  await apiClient.post(`/notifications/channels/${id}/test`)
}

export async function listRules(): Promise<NotificationRule[]> {
  const response = await apiClient.get<NotificationRule[]>('/notifications/rules')
  return response.data
}

export async function createRule(data: CreateRuleRequest): Promise<NotificationRule> {
  const response = await apiClient.post<NotificationRule>('/notifications/rules', data)
  return response.data
}

export async function deleteRule(id: number): Promise<void> {
  await apiClient.delete(`/notifications/rules/${id}`)
}

export interface VapidKeyStatus {
  key: string
  configured: boolean
}

export async function getVapidPublicKey(): Promise<VapidKeyStatus> {
  const response = await apiClient.get<{ public_key: string; configured: boolean }>(
    '/notifications/push/vapid-key',
  )
  return { key: response.data.public_key, configured: response.data.configured }
}

export async function saveVapidKeys(publicKey: string, privateKey: string): Promise<void> {
  await apiClient.put('/notifications/push/vapid-key', {
    public_key: publicKey,
    private_key: privateKey,
  })
}

export async function subscribePush(subscription: PushSubscriptionJSON): Promise<void> {
  await apiClient.post('/notifications/push/subscribe', subscription)
}

export async function unsubscribePush(endpoint: string): Promise<void> {
  await apiClient.post('/notifications/push/unsubscribe', { endpoint })
}

export async function listPushSubscriptions(): Promise<PushSubscriptionInfo[]> {
  const response = await apiClient.get<PushSubscriptionInfo[]>('/notifications/push/subscriptions')
  return response.data
}

export async function listDeliveries(limit?: number): Promise<NotificationDelivery[]> {
  const params = limit ? { limit } : undefined
  const response = await apiClient.get<NotificationDelivery[]>('/notifications/deliveries', {
    params,
  })
  return response.data
}

export interface ValidateSmtpRequest {
  smtp_host: string
  smtp_port: number
  smtp_user: string
  smtp_password: string
  security: string
}

export async function validateSmtp(data: ValidateSmtpRequest): Promise<void> {
  await apiClient.post('/notifications/validate-smtp', data)
}
