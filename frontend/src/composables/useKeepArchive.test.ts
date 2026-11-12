// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../api/appendOnly', () => ({ keepArchive: vi.fn() }))

import { keepArchive } from '../api/appendOnly'
import { useKeepArchive } from './useKeepArchive'
import { useToast } from './useToast'

function toastMessages(): string[] {
  return useToast().toasts.value.map((t) => t.message)
}

describe('useKeepArchive', () => {
  beforeEach(() => {
    useToast().toasts.value = []
    vi.mocked(keepArchive).mockReset().mockResolvedValue(undefined)
  })

  it('cancels the deletion, refreshes the caller and says so', async () => {
    const afterKept = vi.fn()
    const { keep, keeping } = useKeepArchive()

    const pending = keep(7, 'photos-1', afterKept)
    expect(keeping.value).toBe(true)
    await pending

    expect(keepArchive).toHaveBeenCalledWith(7, 'photos-1')
    expect(afterKept).toHaveBeenCalledTimes(1)
    expect(toastMessages()).toContain('photos-1 will be kept.')
    expect(keeping.value).toBe(false)
  })

  it('reports a failure and does not refresh', async () => {
    vi.mocked(keepArchive).mockRejectedValue(new Error('gone already'))
    const afterKept = vi.fn()
    const { keep, keeping } = useKeepArchive()

    await keep(7, 'photos-1', afterKept)

    expect(afterKept).not.toHaveBeenCalled()
    expect(toastMessages()).toContain('gone already')
    expect(keeping.value).toBe(false)
  })
})
