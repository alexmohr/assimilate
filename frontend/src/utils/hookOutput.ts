// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { AgentLogLine } from '../types/generated/AgentLogLine'
import type { HookPhase } from '../types/generated/HookPhase'

/** One line a pre- or post-backup hook printed while the backup was running. */
export type HookOutputEntry = Extract<AgentLogLine, { type: 'hook_output' }>

/** A noisy hook must not grow the page without bound; the newest lines win. */
export const MAX_HOOK_OUTPUT_LINES = 200

/**
 * The agent adds hook output to the same `BackupLog` stream as borg's own
 * lines, tagged `type: "hook_output"`. Anything else is not a hook line.
 */
export function parseHookOutput(raw: string): HookOutputEntry | null {
  try {
    const obj = JSON.parse(raw) as Record<string, unknown>
    if (obj['type'] !== 'hook_output') return null
    if (
      (obj['phase'] !== 'pre_backup' && obj['phase'] !== 'post_backup') ||
      (obj['stream'] !== 'stdout' && obj['stream'] !== 'stderr') ||
      typeof obj['line'] !== 'string'
    ) {
      return null
    }
    return obj as unknown as HookOutputEntry
  } catch {
    return null
  }
}

const PHASE_LABELS: Record<HookPhase, string> = {
  pre_backup: 'pre-backup',
  post_backup: 'post-backup',
}

/** A hook line as plain text for a log that has no room for its phase and stream columns. */
export function formatHookOutput(entry: HookOutputEntry): string {
  return `${PHASE_LABELS[entry.phase]}: ${entry.line}`
}

/** Appends `entry`, dropping the oldest lines past [`MAX_HOOK_OUTPUT_LINES`]. */
export function appendHookOutput(
  lines: readonly HookOutputEntry[],
  entry: HookOutputEntry,
): HookOutputEntry[] {
  return [...lines, entry].slice(-MAX_HOOK_OUTPUT_LINES)
}
