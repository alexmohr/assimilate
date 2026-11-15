// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { HookCommand } from './generated'

/**
 * What a backup reads as files, as the shared Files section edits it: a
 * host's own set, or a schedule's own set used instead of each host's. The
 * line lists are kept as typed text, one entry per line, comments included.
 */
export interface FileSet {
  paths: string
  excludes: string
  /** Exceptions to the excludes, checked first. */
  includes: string
  fileChangePatterns: string
  preCommands: HookCommand[]
  postCommands: HookCommand[]
}
