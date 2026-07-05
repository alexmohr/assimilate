// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import axios from 'axios'
import { logger } from './logger'

interface ServerErrorBody {
  error?: string
  message?: string
  error_id?: string
}

function isServerErrorBody(value: unknown): value is ServerErrorBody {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function formatAxiosErrorMessage(
  detail: string,
  errorId: string | undefined,
  prefix: string,
  context: string | undefined,
  status: number | undefined,
  url: string | undefined,
  error: unknown,
): string {
  const suffix = errorId ? ` (ref: ${errorId})` : ''

  logger.error(prefix, { status, detail, errorId, url, error })

  const msg = `${detail}${suffix}`
  return context ? `${context}: ${msg}` : msg
}

export function extractError(e: unknown, context?: string): string {
  const prefix = context ? `[${context}]` : '[error]'

  if (axios.isAxiosError(e)) {
    const status = e.response?.status
    const url = e.config?.url
    const data = e.response?.data

    let serverMsg: string | undefined
    let errorId: string | undefined

    if (typeof data === 'string' && data.length > 0) {
      serverMsg = data
    } else if (isServerErrorBody(data)) {
      serverMsg = data.error ?? data.message
      errorId = data.error_id
    }

    const detail = serverMsg ?? e.message
    return formatAxiosErrorMessage(detail, errorId, prefix, context, status, url, e)
  }

  if (e instanceof Error) {
    logger.error(prefix, e)
    return context ? `${context}: ${e.message}` : e.message
  }

  logger.error(prefix, e)
  return context ?? 'Unknown error'
}

export async function extractBlobError(e: unknown, context?: string): Promise<string> {
  if (axios.isAxiosError(e) && e.response?.data instanceof Blob) {
    const prefix = context ? `[${context}]` : '[error]'
    const status = e.response.status
    const url = e.config?.url
    const text = await e.response.data.text()

    let serverMsg: string | undefined
    let errorId: string | undefined

    try {
      const j: unknown = JSON.parse(text)
      if (isServerErrorBody(j)) {
        serverMsg = j.error ?? j.message
        errorId = j.error_id
      }
    } catch {
      serverMsg = text || undefined
    }

    const detail = serverMsg ?? e.message
    return formatAxiosErrorMessage(detail, errorId, prefix, context, status, url, e)
  }

  return extractError(e, context)
}
