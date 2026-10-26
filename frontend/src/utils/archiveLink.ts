// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { parentOf } from './archiveSelection'
import type { SymlinkTarget } from '../types/generated'

/**
 * The folder the browser opens to show a link's target: the target itself
 * when it is a folder, the folder holding it otherwise. `null` when the
 * archive does not hold the target, or the server could not say.
 */
export function linkFolder(link: SymlinkTarget): string | null {
  const resolved = link.resolved
  if (resolved.state !== 'in_archive') return null
  return resolved.directory ? resolved.path : parentOf(resolved.path)
}

/** Whether the link's target is a folder of the archive, so opening the link opens it. */
export function linksToFolder(link: SymlinkTarget): boolean {
  return link.resolved.state === 'in_archive' && link.resolved.directory
}

/** The hover text of a link: where it points and what restoring it brings back. */
export function linkTitle(path: string, link: SymlinkTarget): string {
  const where = link.target === undefined ? `/${path}` : `/${path} → ${link.target}`
  switch (link.resolved.state) {
    case 'in_archive':
      return `Symbolic link: ${where}. Restoring it brings back the link, not what it points to.`
    case 'not_in_archive':
      return `Symbolic link: ${where}. Nothing at the target's path is in this archive.`
    case 'unknown':
      return `Symbolic link: ${where}.`
  }
}
