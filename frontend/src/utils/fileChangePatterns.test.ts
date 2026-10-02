// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import shared from '../../../testdata/parity/file_change_patterns.json'

import {
  FileChangeAction,
  parseFileChangePatterns,
  serializeFileChangePatterns,
  type FileChangePatternRow,
} from './fileChangePatterns'

describe('parseFileChangePatterns', () => {
  it('defaults to warn when no action is given', () => {
    const rows = parseFileChangePatterns('*/etc/passwd*\n*/var/log*')
    expect(rows).toEqual([
      { path: '*/etc/passwd*', action: FileChangeAction.Warn },
      { path: '*/var/log*', action: FileChangeAction.Warn },
    ])
  })

  it('parses an explicit trailing action', () => {
    const rows = parseFileChangePatterns('*/tmp* ignore\n*/etc* warn\n*/var/log* fatal')
    expect(rows).toEqual([
      { path: '*/tmp*', action: FileChangeAction.Ignore },
      { path: '*/etc*', action: FileChangeAction.Warn },
      { path: '*/var/log*', action: FileChangeAction.Fatal },
    ])
  })

  it('strips blank lines and comments', () => {
    const rows = parseFileChangePatterns('# comment\n*/tmp* ignore\n\n# another\n*/var/log* fatal')
    expect(rows).toEqual([
      { path: '*/tmp*', action: FileChangeAction.Ignore },
      { path: '*/var/log*', action: FileChangeAction.Fatal },
    ])
  })

  it('returns an empty array for empty input', () => {
    expect(parseFileChangePatterns('')).toEqual([])
  })
})

describe('serializeFileChangePatterns', () => {
  it('omits the action keyword for warn', () => {
    expect(
      serializeFileChangePatterns([{ path: '*/etc/passwd*', action: FileChangeAction.Warn }]),
    ).toBe('*/etc/passwd*')
  })

  it('includes the action keyword for ignore and fatal', () => {
    const raw = serializeFileChangePatterns([
      { path: '*/tmp*', action: FileChangeAction.Ignore },
      { path: '*/var/log*', action: FileChangeAction.Fatal },
    ])
    expect(raw).toBe('*/tmp* ignore\n*/var/log* fatal')
  })

  it('round-trips through parse', () => {
    const raw = '*/tmp* ignore\n*/etc*\n*/var/log* fatal'
    expect(serializeFileChangePatterns(parseFileChangePatterns(raw))).toBe(raw)
  })
})

// The cases crates/server/src/config_assembler.rs runs too, so the two parsers
// cannot drift apart.
describe('shared file-change grammar cases', () => {
  const rows = (cases: { path: string; action: string }[]): FileChangePatternRow[] =>
    cases.map(({ path, action }) => {
      const known = Object.values(FileChangeAction).find((a) => a === action)
      if (known === undefined) throw new Error(`fixture has unknown action ${action}`)
      return { path, action: known }
    })

  it.each(shared.parse)('parses: $name', ({ raw, rows: expected }) => {
    expect(parseFileChangePatterns(raw)).toEqual(rows(expected))
  })

  it.each(shared.serialize)('serializes: $name', ({ raw, rows: given }) => {
    expect(serializeFileChangePatterns(rows(given))).toBe(raw)
  })
})
