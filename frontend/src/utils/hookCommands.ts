// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { HookCommand } from '../types/generated'

/**
 * Mirrors `shared::hooks::MAX_HOOK_COMMAND_TIMEOUT_SECONDS` (24 hours). The
 * server rejects anything above it, so the form caps the field at the same
 * value rather than letting a save fail on a number the input accepted.
 */
export const MAX_HOOK_COMMAND_TIMEOUT_SECONDS = 86_400

/** Builds a hook command that inherits the schedule's hook timeout. */
export function hookCommand(command: string, timeoutSeconds: number | null = null): HookCommand {
  return { command, timeout_seconds: timeoutSeconds }
}
