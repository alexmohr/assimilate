// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/** The OS the install command is written for. */
export type InstallTarget = 'linux' | 'windows'

/** Where an enrollment token is in its life. */
export type EnrollmentTokenStatus = 'pending' | 'used' | 'expired' | 'revoked'

export interface EnrollmentTokenRow {
  id: number
  prefix: string
  created_by: string
  hostname: string | null
  status: EnrollmentTokenStatus
  detail: string
}
