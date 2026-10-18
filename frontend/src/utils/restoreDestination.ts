// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * The restore basket's destination field. Mirrors the checks the server and
 * the agent apply (`shared::restore::RestoreDestination`), so a mistake is
 * shown beside the field instead of coming back from the request.
 */

/** Longest destination accepted, in characters: Linux's `PATH_MAX`. */
const MAX_DESTINATION_LENGTH = 4096

/** An empty field restores to the original locations, so it is `null`. */
export type DestinationCheck =
  | { ok: true; destination: string | null }
  | { ok: false; error: string }

// Matches any C0 control character or DEL; the agent refuses them too.
// eslint-disable-next-line no-control-regex
const CONTROL_CHARACTER = /[\u0000-\u001f\u007f]/

export function checkDestination(raw: string): DestinationCheck {
  const trimmed = raw.trim()
  if (trimmed === '') return { ok: true, destination: null }
  if (trimmed.length > MAX_DESTINATION_LENGTH) {
    return { ok: false, error: `Use at most ${MAX_DESTINATION_LENGTH} characters.` }
  }
  if (CONTROL_CHARACTER.test(trimmed)) {
    return { ok: false, error: 'The path must not contain control characters.' }
  }
  if (!trimmed.startsWith('/')) {
    return { ok: false, error: 'Use an absolute path, starting with /.' }
  }
  const segments = trimmed.split('/').filter((segment) => segment.length > 0)
  if (segments.some((segment) => /^\.\.?$/.test(segment))) {
    return { ok: false, error: 'The path must not contain . or .. segments.' }
  }
  return { ok: true, destination: segments.length === 0 ? null : `/${segments.join('/')}` }
}

/**
 * Where `archivePath` (as borg stores it, no leading slash) ends up on the
 * host: under `destination` with its archive structure, or at its original
 * location when there is no destination.
 */
export function restoredLocation(destination: string | null, archivePath: string): string {
  const path = archivePath.replace(/^\/+/, '')
  if (destination === null) return `/${path}`
  return path === '' ? destination : `${destination}/${path}`
}
