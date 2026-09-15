// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { readStorage, writeStorage, removeStorage } from './storage'

describe('storage', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('round-trips a value through write/read/remove', () => {
    writeStorage('assimilate-test-key', 'value')
    expect(readStorage('assimilate-test-key')).toBe('value')

    removeStorage('assimilate-test-key')
    expect(readStorage('assimilate-test-key')).toBeUndefined()
  })

  it('returns undefined for a key that was never set', () => {
    expect(readStorage('assimilate-test-missing')).toBeUndefined()
  })

  describe('when localStorage throws', () => {
    let getItemSpy: ReturnType<typeof vi.spyOn>
    let setItemSpy: ReturnType<typeof vi.spyOn>
    let removeItemSpy: ReturnType<typeof vi.spyOn>

    beforeEach(() => {
      getItemSpy = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
        throw new DOMException('blocked', 'SecurityError')
      })
      setItemSpy = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
        throw new DOMException('QuotaExceededError', 'QuotaExceededError')
      })
      removeItemSpy = vi.spyOn(Storage.prototype, 'removeItem').mockImplementation(() => {
        throw new DOMException('blocked', 'SecurityError')
      })
    })

    afterEach(() => {
      getItemSpy.mockRestore()
      setItemSpy.mockRestore()
      removeItemSpy.mockRestore()
    })

    it('readStorage swallows the throw and returns undefined instead of crashing the caller', () => {
      expect(() => readStorage('assimilate-test-key')).not.toThrow()
      expect(readStorage('assimilate-test-key')).toBeUndefined()
    })

    it('writeStorage swallows the throw instead of crashing the caller', () => {
      expect(() => writeStorage('assimilate-test-key', 'value')).not.toThrow()
    })

    it('removeStorage swallows the throw instead of crashing the caller', () => {
      expect(() => removeStorage('assimilate-test-key')).not.toThrow()
    })
  })
})
