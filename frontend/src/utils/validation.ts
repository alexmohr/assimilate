// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { HookCommand } from '../types/generated'

export function parseLines(text: string): string[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0)
}

/**
 * Drops blank hook-command entries (e.g. a "+ Add command" row left empty)
 * before saving. Never trims the surviving entries' content - a command is a
 * whole script, and leading whitespace can be meaningful indentation.
 */
export function dropBlankCommands(commands: HookCommand[]): HookCommand[] {
  return commands.filter((c) => c.command.trim().length > 0)
}

export function validatePassword(newPassword: string, confirmPassword: string): string | null {
  if (newPassword.length < 8) {
    return 'Password must be at least 8 characters'
  }
  if (newPassword !== confirmPassword) {
    return 'Passwords do not match'
  }
  return null
}
