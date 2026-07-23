// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { vi } from 'vitest'

export function mockTimezone(): {
  useTimezone: ReturnType<typeof vi.fn>
  getConfiguredTimezone: ReturnType<typeof vi.fn>
} {
  return {
    useTimezone: vi.fn(),
    getConfiguredTimezone: vi.fn().mockReturnValue(undefined),
  }
}

export function mockApiClient(): { apiClient: { get: ReturnType<typeof vi.fn> } } {
  return { apiClient: { get: vi.fn() } }
}
