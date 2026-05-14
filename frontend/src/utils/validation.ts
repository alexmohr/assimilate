// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

export function parseLines(text: string): string[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0)
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
