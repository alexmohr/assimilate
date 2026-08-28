// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { dropBlankCommands } from './validation'

describe('dropBlankCommands', () => {
  it('drops blank and whitespace-only entries', () => {
    expect(dropBlankCommands(['echo one', '', '   ', 'echo two'])).toEqual(['echo one', 'echo two'])
  })

  it('keeps leading whitespace in a surviving entry, since it may be meaningful indentation', () => {
    const script = '  if true; then\n    echo indented\n  fi'
    expect(dropBlankCommands([script])).toEqual([script])
  })

  it('returns an empty array unchanged', () => {
    expect(dropBlankCommands([])).toEqual([])
  })
})
