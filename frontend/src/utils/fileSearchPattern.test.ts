// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { fileNameGlob } from './fileSearchPattern'

describe('fileNameGlob', () => {
  it('matches plain text anywhere in an entry name, at any depth', () => {
    expect(fileNameGlob('nginx')).toBe('**/*nginx*')
  })

  it('trims the text first', () => {
    expect(fileNameGlob('  notes.md ')).toBe('**/*notes.md*')
  })

  it('keeps a pattern that is already a glob', () => {
    expect(fileNameGlob('*.sql')).toBe('*.sql')
    expect(fileNameGlob('home/**/*.conf')).toBe('home/**/*.conf')
    expect(fileNameGlob('IMG_44[0-9]?.HEIC')).toBe('IMG_44[0-9]?.HEIC')
  })
})
