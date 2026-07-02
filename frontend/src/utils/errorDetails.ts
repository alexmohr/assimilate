// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

const STORAGE_KEY = 'assimilate:last-error'

export interface ErrorDetails {
  source: 'frontend' | 'backend'
  name?: string
  message: string
  stack?: string
}

/**
 * Stash error details in sessionStorage so the full-page error view can read
 * them after a router navigation. Query params are unsuitable here since
 * stack traces can exceed practical URL length limits.
 */
export function storeErrorDetails(details: ErrorDetails): void {
  try {
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify(details))
  } catch {
    // sessionStorage may be unavailable (e.g. private browsing quota); details are best-effort.
  }
}

/** Read and clear the stashed error details, if any. */
export function consumeErrorDetails(): ErrorDetails | undefined {
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY)
    if (!raw) {
      return undefined
    }
    sessionStorage.removeItem(STORAGE_KEY)
    return JSON.parse(raw) as ErrorDetails
  } catch {
    return undefined
  }
}
