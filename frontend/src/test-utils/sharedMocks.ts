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

export function mockFormatBytes(): { formatBytes: (bytes: number) => string } {
  return { formatBytes: (bytes: number): string => `${bytes} B` }
}

export function mockErrorUtils(): {
  extractError: (e: unknown) => string
  extractBlobError: (e: unknown) => Promise<string>
} {
  return {
    extractError: (_e: unknown): string => 'API error',
    extractBlobError: async (_e: unknown): Promise<string> => 'API error',
  }
}
