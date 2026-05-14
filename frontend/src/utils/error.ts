// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import axios from 'axios'
import { logger } from './logger'

/**
 * Extract a human-readable error message from an unknown caught value.
 * Always logs the full error to the console for debugging.
 */
export function extractError(e: unknown, context?: string): string {
  const prefix = context ? `[${context}]` : '[error]'

  if (axios.isAxiosError(e)) {
    const status = e.response?.status
    const data = e.response?.data as
      | { error?: string; message?: string; error_id?: string }
      | string
      | undefined

    let serverMsg: string | undefined
    let errorId: string | undefined

    if (typeof data === 'string' && data.length > 0) {
      serverMsg = data
    } else if (typeof data === 'object' && data !== null) {
      serverMsg = data.error ?? data.message
      errorId = data.error_id
    }

    const detail = serverMsg ?? e.message
    const suffix = errorId ? ` (ref: ${errorId})` : ''

    logger.error(prefix, { status, detail, errorId, url: e.config?.url, error: e })

    const msg = `${detail}${suffix}`
    return context ? `${context}: ${msg}` : msg
  }

  if (e instanceof Error) {
    logger.error(prefix, e)
    return context ? `${context}: ${e.message}` : e.message
  }

  logger.error(prefix, e)
  return context ?? 'Unknown error'
}
