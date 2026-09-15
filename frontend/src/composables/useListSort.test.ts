// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, beforeEach } from 'vitest'
import { nextTick } from 'vue'
import { useListSort } from './useListSort'

type Field = 'name' | 'size' | 'age'
const FIELDS: readonly Field[] = ['name', 'size', 'age']

describe('useListSort', () => {
  it('starts on the field it was given, ascending', () => {
    const sort = useListSort<Field>('name')
    expect(sort.field.value).toBe('name')
    expect(sort.direction.value).toBe('asc')
    expect(sort.sign()).toBe(1)
  })

  it('honours an explicit initial direction', () => {
    const sort = useListSort<Field>('age', 'desc')
    expect(sort.direction.value).toBe('desc')
    expect(sort.sign()).toBe(-1)
  })

  it('flips direction when the active field is toggled again', () => {
    const sort = useListSort<Field>('name')
    sort.toggle('name')
    expect(sort.field.value).toBe('name')
    expect(sort.direction.value).toBe('desc')
    sort.toggle('name')
    expect(sort.direction.value).toBe('asc')
  })

  it('starts a newly selected field ascending, whatever the previous direction', () => {
    const sort = useListSort<Field>('name')
    sort.toggle('name')
    expect(sort.direction.value).toBe('desc')

    sort.toggle('size')
    expect(sort.field.value).toBe('size')
    expect(sort.direction.value).toBe('asc')
  })

  it('gives a comparator sign that flips with the direction', () => {
    const sort = useListSort<Field>('size')
    const rows = [3, 1, 2]

    const ascending = [...rows].sort((a, b) => (a - b) * sort.sign())
    expect(ascending).toEqual([1, 2, 3])

    sort.toggle('size')
    const descending = [...rows].sort((a, b) => (a - b) * sort.sign())
    expect(descending).toEqual([3, 2, 1])
  })

  describe('persistence', () => {
    beforeEach(() => {
      localStorage.clear()
    })

    it('does not touch storage when no persist option is given', async () => {
      const sort = useListSort<Field>('name')
      sort.toggle('size')
      await nextTick()
      expect(localStorage.getItem('assimilate-test-sort-field')).toBeNull()
    })

    it('restores a previously persisted field and direction', () => {
      localStorage.setItem('assimilate-test-sort-field', 'age')
      localStorage.setItem('assimilate-test-sort-dir', 'desc')
      const sort = useListSort<Field>('name', 'asc', {
        key: 'assimilate-test-sort',
        fields: FIELDS,
      })
      expect(sort.field.value).toBe('age')
      expect(sort.direction.value).toBe('desc')
    })

    it('falls back to the given initial when storage holds an unknown field', () => {
      localStorage.setItem('assimilate-test-sort-field', 'unknown-column')
      const sort = useListSort<Field>('name', 'asc', {
        key: 'assimilate-test-sort',
        fields: FIELDS,
      })
      expect(sort.field.value).toBe('name')
    })

    it('persists a toggle to a new field, resetting the persisted direction to ascending', async () => {
      const sort = useListSort<Field>('name', 'asc', {
        key: 'assimilate-test-sort',
        fields: FIELDS,
      })
      sort.toggle('name')
      await nextTick()
      expect(localStorage.getItem('assimilate-test-sort-dir')).toBe('desc')

      sort.toggle('size')
      await nextTick()
      expect(localStorage.getItem('assimilate-test-sort-field')).toBe('size')
      expect(localStorage.getItem('assimilate-test-sort-dir')).toBe('asc')
    })

    it('persists a direction flip on the same field', async () => {
      const sort = useListSort<Field>('name', 'asc', {
        key: 'assimilate-test-sort',
        fields: FIELDS,
      })
      sort.toggle('name')
      await nextTick()
      expect(localStorage.getItem('assimilate-test-sort-dir')).toBe('desc')
    })
  })
})
