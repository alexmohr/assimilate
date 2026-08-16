// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { validateSmtp } from '../api/notifications'
import { extractError } from './error'
import type { EmailConfig } from '../types/notifications'

export interface SmtpVerdict {
  success: boolean
  message: string
}

/**
 * Checks whether an SMTP config can actually log in.
 *
 * Deliberately a plain function over the config rather than a method on the
 * fields component: the dialogs that gate saving on this are wizards, and the
 * SMTP fields are only rendered on their first step. Reaching the check
 * through a component ref meant the ref was null by the time the user pressed
 * Create, so the gate failed closed and no email channel could be created at
 * all. Keeping it here makes the check independent of what happens to be on
 * screen.
 */
export async function validateEmailConfig(cfg: EmailConfig): Promise<SmtpVerdict> {
  try {
    await validateSmtp({
      smtp_host: cfg.smtp_host,
      smtp_port: cfg.smtp_port,
      smtp_user: cfg.smtp_user,
      smtp_password: cfg.smtp_password,
      security: cfg.security ?? 'starttls',
    })
    return { success: true, message: 'SMTP login successful' }
  } catch (e: unknown) {
    return { success: false, message: extractError(e) }
  }
}
