// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { destinationOf, folderError, targetPath } from './restoreDestination'

describe('restoreDestination', () => {
  it('sends / for the original locations and the folder otherwise', () => {
    expect(targetPath({ kind: 'original' })).toBe('/')
    expect(targetPath({ kind: 'folder', path: '/srv/restore' })).toBe('/srv/restore')
  })

  it('accepts an absolute folder', () => {
    expect(folderError('/srv/restore')).toBeNull()
    expect(folderError('  /srv/restore/  ')).toBeNull()
  })

  it.each([
    ['', 'Enter the folder'],
    ['srv/restore', 'absolute path'],
    ['/srv/../etc', '. or ..'],
    ['/srv/./restore', '. or ..'],
    ['/srv/re\nstore', 'control character'],
    ['/', 'Original paths'],
    ['///', 'Original paths'],
  ])('refuses %j', (input, reason) => {
    expect(folderError(input)).toContain(reason)
  })

  it('keeps the full archive path under the folder', () => {
    expect(destinationOf('etc/nginx/nginx.conf', { kind: 'original' })).toBe(
      '/etc/nginx/nginx.conf',
    )
    expect(destinationOf('/etc/hosts', { kind: 'folder', path: '/srv/restore/' })).toBe(
      '/srv/restore/etc/hosts',
    )
  })
})
