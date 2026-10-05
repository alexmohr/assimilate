// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useToast } from './useToast'

/**
 * The auto-dismiss timer used to be covered only when some other spec that
 * raised a real toast happened to live past its 4-6 seconds - so whether
 * `removeToast` counted as covered depended on how long unrelated tests took
 * (#353). Fake timers make it a fact of this file instead.
 */
describe('useToast', () => {
  const toast = useToast()

  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    toast.toasts.value.splice(0)
    vi.useRealTimers()
  })

  function messages(): string[] {
    return toast.toasts.value.map((t) => t.message)
  }

  it('dismisses each kind of toast once its own duration has passed', () => {
    toast.success('saved')
    toast.info('fyi')
    toast.warning('careful')
    toast.error('failed')
    expect(messages()).toEqual(['saved', 'fyi', 'careful', 'failed'])

    vi.advanceTimersByTime(3999)
    expect(messages()).toEqual(['saved', 'fyi', 'careful', 'failed'])

    vi.advanceTimersByTime(1)
    expect(messages()).toEqual(['careful', 'failed'])

    vi.advanceTimersByTime(1000)
    expect(messages()).toEqual(['failed'])

    vi.advanceTimersByTime(1000)
    expect(messages()).toEqual([])
  })

  it('records the type and duration each helper stands for', () => {
    toast.success('a')
    toast.info('b')
    toast.warning('c')
    toast.error('d')

    expect(toast.toasts.value.map(({ type, duration }) => ({ type, duration }))).toEqual([
      { type: 'success', duration: 4000 },
      { type: 'info', duration: 4000 },
      { type: 'warning', duration: 5000 },
      { type: 'error', duration: 6000 },
    ])
  })

  it('removes a dismissed toast immediately and leaves its timer harmless', () => {
    toast.success('first')
    toast.success('second')
    const [first] = toast.toasts.value
    if (!first) throw new Error('expected the first toast to be showing')

    toast.remove(first.id)
    expect(messages()).toEqual(['second'])

    // The dismissed toast's own timer still fires, and finds nothing to remove.
    vi.advanceTimersByTime(4000)
    expect(messages()).toEqual([])
  })

  it('ignores an id that is not showing', () => {
    toast.info('kept')

    toast.remove(-1)

    expect(messages()).toEqual(['kept'])
  })
})
