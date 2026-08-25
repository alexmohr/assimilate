// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * The one line of a borg `--progress` stream that carries structured data;
 * every other line is free-form log text. Shared by the agent and schedule
 * detail views, which both parse a live `BackupLog` stream into this shape.
 */
export interface BorgArchiveProgress {
  type: 'archive_progress'
  nfiles: number
  original_size: number
  path: string
}

export function parseArchiveProgress(raw: string): BorgArchiveProgress | null {
  try {
    const obj = JSON.parse(raw) as Record<string, unknown>
    if (obj['type'] !== 'archive_progress') return null
    // Borg's final progress line before completion (`"finished": true`) omits
    // nfiles/original_size/path - ignore it and keep the last known progress.
    if (
      typeof obj['nfiles'] !== 'number' ||
      typeof obj['original_size'] !== 'number' ||
      typeof obj['path'] !== 'string'
    ) {
      return null
    }
    return obj as unknown as BorgArchiveProgress
  } catch {
    return null
  }
}
