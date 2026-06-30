// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn() },
}))

import { useAsyncAction } from './useAsyncAction'

describe('useAsyncAction', () => {
  it('toggles loading around a successful run and returns the value', async () => {
    const { loading, error, run } = useAsyncAction()
    expect(loading.value).toBe(false)

    let observedDuringRun = false
    const promise = run(async () => {
      observedDuringRun = loading.value
      return 42
    })
    const result = await promise

    expect(observedDuringRun).toBe(true)
    expect(result).toBe(42)
    expect(loading.value).toBe(false)
    expect(error.value).toBeNull()
  })

  it('clears a previous error at the start of a new run', async () => {
    const { error, run } = useAsyncAction()

    await run(async () => {
      throw new Error('boom')
    })
    expect(error.value).toBe('boom')

    await run(async () => 'ok')
    expect(error.value).toBeNull()
  })

  it('captures the error message and returns undefined on throw', async () => {
    const { loading, error, run } = useAsyncAction()

    const result = await run(async () => {
      throw new Error('something failed')
    })

    expect(result).toBeUndefined()
    expect(error.value).toBe('something failed')
    expect(loading.value).toBe(false)
  })

  it('prefixes the error message with the provided context', async () => {
    const { error, run } = useAsyncAction('Load schedules')

    await run(async () => {
      throw new Error('network down')
    })

    expect(error.value).toBe('Load schedules: network down')
  })
})
