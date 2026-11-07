// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/** Characters that make the text a glob pattern of its own. */
const GLOB_CHARS = /[*?[\]]/

/**
 * The borg `sh:` pattern a search across archives runs for `text`.
 *
 * The list search offers "Find files named ...", so plain text matches any
 * entry whose name contains it, at any depth: `nginx` finds
 * `etc/nginx/nginx.conf` and `etc/nginx`. Text that already holds a glob
 * character (`*.sql`, or a path with `**` in it) is taken as the pattern it is.
 */
export function fileNameGlob(text: string): string {
  const trimmed = text.trim()
  return GLOB_CHARS.test(trimmed) ? trimmed : `**/*${trimmed}*`
}
