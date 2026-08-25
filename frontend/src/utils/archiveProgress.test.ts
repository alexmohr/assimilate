// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { parseArchiveProgress } from './archiveProgress'

describe('parseArchiveProgress', () => {
  it('parses a valid archive_progress line', () => {
    const line = JSON.stringify({
      type: 'archive_progress',
      nfiles: 1234,
      original_size: 5368709120,
      path: '/home/user/important.txt',
    })
    expect(parseArchiveProgress(line)).toEqual({
      type: 'archive_progress',
      nfiles: 1234,
      original_size: 5368709120,
      path: '/home/user/important.txt',
    })
  })

  it('returns null for a differently-typed JSON line', () => {
    expect(parseArchiveProgress(JSON.stringify({ type: 'log_message', message: 'hi' }))).toBeNull()
  })

  it('returns null for non-JSON input', () => {
    expect(parseArchiveProgress('Creating archive server-daily-2026-06-26...')).toBeNull()
  })

  it('returns null for the final "finished" progress line, which omits nfiles/original_size/path', () => {
    const line = JSON.stringify({ type: 'archive_progress', finished: true, time: 1750000000 })
    expect(parseArchiveProgress(line)).toBeNull()
  })
})
