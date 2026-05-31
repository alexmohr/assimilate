// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'

function patternsToText(patterns: string[]): string {
  return patterns.join('\n')
}

function textToPatterns(t: string): string[] {
  return t
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0)
}

interface GlobalExclude {
  id: number
  pattern: string
  sort_order: number
}

function computeSaveDiff(
  text: string,
  existing: GlobalExclude[],
): { toDelete: GlobalExclude[]; toAdd: string[]; desired: string[] } {
  const desired = [...new Set(textToPatterns(text))]
  const toDelete = existing.filter((e) => !desired.includes(e.pattern))
  const toAdd = desired.filter((p) => !existing.some((e) => e.pattern === p))
  return { toDelete, toAdd, desired }
}

describe('ExcludesView helpers', () => {
  describe('textToPatterns', () => {
    it('splits lines and trims whitespace', () => {
      expect(textToPatterns('  foo  \n  bar  \n')).toEqual(['foo', 'bar'])
    })

    it('filters empty lines', () => {
      expect(textToPatterns('\n\nfoo\n\nbar\n\n')).toEqual(['foo', 'bar'])
    })

    it('returns empty array for empty string', () => {
      expect(textToPatterns('')).toEqual([])
    })
  })

  describe('patternsToText', () => {
    it('joins patterns with newlines', () => {
      expect(patternsToText(['foo', 'bar'])).toBe('foo\nbar')
    })

    it('returns empty string for empty array', () => {
      expect(patternsToText([])).toBe('')
    })
  })

  describe('computeSaveDiff', () => {
    it('deduplicates patterns in text', () => {
      const { desired } = computeSaveDiff('foo\nbar\nfoo\nbaz\nbar', [])
      expect(desired).toEqual(['foo', 'bar', 'baz'])
    })

    it('identifies patterns to add', () => {
      const existing: GlobalExclude[] = [{ id: 1, pattern: 'foo', sort_order: 0 }]
      const { toAdd } = computeSaveDiff('foo\nbar\nbaz', existing)
      expect(toAdd).toEqual(['bar', 'baz'])
    })

    it('identifies patterns to delete', () => {
      const existing: GlobalExclude[] = [
        { id: 1, pattern: 'foo', sort_order: 0 },
        { id: 2, pattern: 'bar', sort_order: 1 },
        { id: 3, pattern: 'baz', sort_order: 2 },
      ]
      const { toDelete } = computeSaveDiff('foo\nbaz', existing)
      expect(toDelete).toEqual([{ id: 2, pattern: 'bar', sort_order: 1 }])
    })

    it('handles no changes', () => {
      const existing: GlobalExclude[] = [
        { id: 1, pattern: 'foo', sort_order: 0 },
        { id: 2, pattern: 'bar', sort_order: 1 },
      ]
      const { toDelete, toAdd } = computeSaveDiff('foo\nbar', existing)
      expect(toDelete).toEqual([])
      expect(toAdd).toEqual([])
    })

    it('deduplicates before diffing against existing', () => {
      const existing: GlobalExclude[] = [{ id: 1, pattern: 'foo', sort_order: 0 }]
      const { toAdd, desired } = computeSaveDiff('foo\nbar\nbar\nbar', existing)
      expect(desired).toEqual(['foo', 'bar'])
      expect(toAdd).toEqual(['bar'])
    })

    it('handles empty text (deletes all)', () => {
      const existing: GlobalExclude[] = [
        { id: 1, pattern: 'foo', sort_order: 0 },
        { id: 2, pattern: 'bar', sort_order: 1 },
      ]
      const { toDelete, toAdd } = computeSaveDiff('', existing)
      expect(toDelete).toEqual(existing)
      expect(toAdd).toEqual([])
    })

    it('handles empty existing (adds all)', () => {
      const { toDelete, toAdd } = computeSaveDiff('foo\nbar', [])
      expect(toDelete).toEqual([])
      expect(toAdd).toEqual(['foo', 'bar'])
    })
  })
})
