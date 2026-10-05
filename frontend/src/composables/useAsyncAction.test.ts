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

  describe('runLatest', () => {
    /** A promise the test settles by hand, to interleave two loads. */
    function deferred<T>(): {
      promise: Promise<T>
      resolve: (v: T) => void
      reject: (e: unknown) => void
    } {
      let resolve!: (v: T) => void
      let reject!: (e: unknown) => void
      const promise = new Promise<T>((res, rej) => {
        resolve = res
        reject = rej
      })
      return { promise, resolve, reject }
    }

    it('behaves like run for a single call', async () => {
      const { loading, error, runLatest } = useAsyncAction()
      let current: boolean | undefined
      const result = await runLatest(async (isCurrent) => {
        current = isCurrent()
        return 7
      })
      expect(result).toBe(7)
      expect(current).toBe(true)
      expect(loading.value).toBe(false)
      expect(error.value).toBeNull()
    })

    it('keeps loading until the newest call settles, even if an older one settles last', async () => {
      const { loading, runLatest } = useAsyncAction()
      const first = deferred<void>()
      const second = deferred<void>()
      const firstRun = runLatest(() => first.promise)
      const secondRun = runLatest(() => second.promise)

      second.resolve()
      await secondRun
      expect(loading.value).toBe(false)

      // A stale load settling afterwards must not flip anything back.
      first.resolve()
      await firstRun
      expect(loading.value).toBe(false)
    })

    it('does not let an older call clear loading while a newer one is in flight', async () => {
      const { loading, runLatest } = useAsyncAction()
      const first = deferred<void>()
      const second = deferred<void>()
      const firstRun = runLatest(() => first.promise)
      const secondRun = runLatest(() => second.promise)

      first.resolve()
      await firstRun
      expect(loading.value).toBe(true)

      second.resolve()
      await secondRun
      expect(loading.value).toBe(false)
    })

    it('drops the error of a superseded call', async () => {
      const { error, runLatest } = useAsyncAction()
      const first = deferred<void>()
      const firstRun = runLatest(() => first.promise)
      await runLatest(async () => undefined)

      first.reject(new Error('old page failed'))
      await firstRun
      expect(error.value).toBeNull()
    })

    it('reports isCurrent false to a superseded call', async () => {
      const { runLatest } = useAsyncAction()
      const first = deferred<void>()
      let staleCheck: (() => boolean) | undefined
      const firstRun = runLatest(async (isCurrent) => {
        staleCheck = isCurrent
        await first.promise
      })
      await runLatest(async () => undefined)
      first.resolve()
      await firstRun
      expect(staleCheck?.()).toBe(false)
    })

    it('hands out guards that go stale once the next runLatest starts', async () => {
      const { runLatest, latestGuard } = useAsyncAction()
      const guard = latestGuard()
      expect(guard()).toBe(true)
      await runLatest(async () => undefined)
      expect(guard()).toBe(false)
      expect(latestGuard()()).toBe(true)
    })
  })
})
