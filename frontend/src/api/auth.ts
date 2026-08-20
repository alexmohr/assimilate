// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  RefreshSessionResponse,
  SessionListResponse,
  SessionResponse,
  TotpSetupResponse,
  TotpVerifyResponse,
} from '../types/generated'

export interface AuthUser {
  id: number
  username: string
  role: string
  must_change_password: boolean
  created_at: string
  last_login_at: string | null
  can_upgrade_agent: boolean
  totp_enabled?: boolean
}

export interface CurrentUserResponse extends AuthUser {
  session_expires_at: string | null
  remember_me: boolean
  totp_enabled: boolean
}

export interface LoginResult {
  user: AuthUser
  session_expires_at: string
  remember_me: boolean
  totp_required: boolean
  temp_token: string | null
}

export interface TotpLoginResult {
  user: AuthUser
  session_expires_at: string
  remember_me: boolean
}

// The backend serializes `PreferencesResponse` with `#[serde(transparent)]`,
// so the wire payload is the raw preferences object, not `{ inner: ... }` as
// the generated (ts-rs) binding suggests.
export interface UserPreferences {
  theme?: string
}

export async function refreshSession(): Promise<RefreshSessionResponse> {
  const response = await apiClient.post<RefreshSessionResponse>('/auth/refresh')
  return response.data
}

export async function getCurrentUser(): Promise<CurrentUserResponse> {
  const response = await apiClient.get<CurrentUserResponse>('/auth/me')
  return response.data
}

export async function login(
  username: string,
  password: string,
  remember: boolean,
): Promise<LoginResult> {
  const response = await apiClient.post<LoginResult>('/auth/login', {
    username,
    password,
    remember_me: remember,
  })
  return response.data
}

export async function verifyTotpLogin(
  code: string,
  tempToken: string | null,
  recovery: boolean,
): Promise<TotpLoginResult> {
  const endpoint = recovery ? '/auth/totp/recovery' : '/auth/totp/verify-login'
  const response = await apiClient.post<TotpLoginResult>(endpoint, {
    code,
    temp_token: tempToken,
  })
  return response.data
}

export async function changePassword(newPassword: string): Promise<void> {
  await apiClient.post('/auth/change-password', { new_password: newPassword })
}

export async function logout(): Promise<void> {
  await apiClient.post('/auth/logout')
}

export async function setupTotp(): Promise<TotpSetupResponse> {
  const response = await apiClient.post<TotpSetupResponse>('/auth/totp/setup')
  return response.data
}

export async function verifyTotp(code: string): Promise<TotpVerifyResponse> {
  const response = await apiClient.post<TotpVerifyResponse>('/auth/totp/verify', { code })
  return response.data
}

export async function disableTotp(password: string): Promise<TotpVerifyResponse> {
  const response = await apiClient.post<TotpVerifyResponse>('/auth/totp/disable', { password })
  return response.data
}

export async function listSessions(): Promise<SessionResponse[]> {
  const response = await apiClient.get<SessionListResponse>('/auth/sessions')
  return response.data.sessions
}

export async function revokeSession(id: string): Promise<void> {
  await apiClient.delete(`/auth/sessions/${id}`)
}

export async function getPreferences(): Promise<UserPreferences> {
  const response = await apiClient.get<UserPreferences>('/auth/preferences')
  return response.data
}

export async function updatePreferences(theme: string): Promise<void> {
  await apiClient.put('/auth/preferences', { theme })
}
