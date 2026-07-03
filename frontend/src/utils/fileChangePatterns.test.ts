// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'

import {
  FileChangeAction,
  parseFileChangePatterns,
  serializeFileChangePatterns,
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
