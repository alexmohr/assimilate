// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * Where a restore writes on the agent: back where every path was backed up
 * from, or under one folder, each path keeping its full archive path inside it
 * (`etc/nginx/nginx.conf` restored into `/srv/restore` lands at
 * `/srv/restore/etc/nginx/nginx.conf`).
 */
export type RestoreLocation = { kind: 'original' } | { kind: 'folder'; path: string }

/** The `target_path` the restore request takes: `/` means the original locations. */
export function targetPath(location: RestoreLocation): string {
  return location.kind === 'original' ? '/' : location.path
}

/** A path segment that names a directory, or one of the two that move relative to it. */
function segmentKind(segment: string): 'self' | 'parent' | 'name' {
  if (segment === '.') return 'self'
  if (segment === '..') return 'parent'
  return 'name'
}

/**
 * Why `input` is not a usable folder, or null when it is. The agent runs borg
 * in this directory, so it has to be an absolute path that cannot climb out
 * of itself; `/` itself is what "original locations" already means.
 */
export function folderError(input: string): string | null {
  const path = input.trim()
  if (path === '') return 'Enter the folder to restore into.'
  if (!path.startsWith('/')) return 'Use an absolute path, starting with /.'
  const isControl = (ch: string): boolean => ch.charCodeAt(0) < 0x20 || ch.charCodeAt(0) === 0x7f
  if ([...path].some(isControl)) return 'The path contains a control character.'
  if (path.split('/').some((segment) => segmentKind(segment) !== 'name')) {
    return 'The path must not contain . or .. segments.'
  }
  if (path.replace(/\/+$/, '') === '') return 'Choose "Original paths" to restore to /.'
  return null
}

/** Where `archivePath` ends up on the agent, for the preview. */
export function destinationOf(archivePath: string, location: RestoreLocation): string {
  const relative = archivePath.replace(/^\/+/, '')
  if (location.kind === 'original') return `/${relative}`
  return `${location.path.replace(/\/+$/, '')}/${relative}`
}
