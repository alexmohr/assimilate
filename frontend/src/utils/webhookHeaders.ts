// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { WebhookHeaderStatus } from '../types/generated'

/**
 * One row of the webhook header editor.
 *
 * The server stores every header value encrypted and never sends it back, so
 * a saved header arrives as a name only. `savedName` remembers which saved
 * header a row started as: while its name still matches, a blank value means
 * "keep the saved one". Renaming the row makes it a new header, whose blank
 * value really is empty.
 */
export interface WebhookHeaderRow {
  id: number
  name: string
  value: string
  savedName: string | null
}

// A plain counter, for the same reason as CommandListEditor's: the id only
// keys sibling rows for Vue, and randomUUID is unavailable over plain HTTP.
let nextRowId = 0

export function newHeaderRow(): WebhookHeaderRow {
  return { id: nextRowId++, name: '', value: '', savedName: null }
}

/** Editor rows for a saved channel's headers, values blank. */
export function rowsFromSaved(headers: readonly WebhookHeaderStatus[]): WebhookHeaderRow[] {
  return headers.map((header) => ({
    id: nextRowId++,
    name: header.name,
    value: '',
    savedName: header.has_value ? header.name : null,
  }))
}

/** Whether leaving this row's value blank keeps a saved value. */
export function keepsSavedValue(row: WebhookHeaderRow): boolean {
  return row.savedName !== null && row.name.trim().toLowerCase() === row.savedName.toLowerCase()
}

/**
 * The request's `headers` object: the channel's complete header set. A blank
 * value is sent as `null`, which keeps a saved value under that name and
 * otherwise stores the header empty. Rows without a name are dropped.
 */
export function headersForRequest(
  rows: readonly WebhookHeaderRow[],
): Record<string, string | null> {
  return Object.fromEntries(
    rows
      .map((row) => ({ name: row.name.trim(), value: row.value }))
      .filter((row) => row.name !== '')
      .map((row) => [row.name, row.value === '' ? null : row.value]),
  )
}

const WEB_PROTOCOLS: readonly string[] = ['http:', 'https:']

/** The URL's scheme, host and port, or null for anything a webhook cannot use. */
function urlOrigin(url: string): string | null {
  try {
    const parsed = new URL(url.trim())
    return WEB_PROTOCOLS.includes(parsed.protocol) ? parsed.origin : null
  } catch {
    return null
  }
}

/**
 * Names of the saved header values that must be typed again before saving.
 *
 * A saved value is only ever sent to the scheme, host and port it was entered
 * for - otherwise editing the URL would hand a stored token to whatever server
 * the new URL names. The server enforces this; the dialog just says so first.
 */
export function headersNeedingReentry(
  savedUrl: string | undefined,
  url: string,
  rows: readonly WebhookHeaderRow[],
): string[] {
  if (savedUrl === undefined) return []
  const savedOrigin = urlOrigin(savedUrl)
  if (savedOrigin !== null && savedOrigin === urlOrigin(url)) return []
  return rows.filter((row) => keepsSavedValue(row) && row.value === '').map((row) => row.name)
}
