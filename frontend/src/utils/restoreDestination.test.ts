// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { checkDestination, restoredLocation } from './restoreDestination'

describe('checkDestination', () => {
  it('reads an empty field, or /, as the original locations', () => {
    expect(checkDestination('')).toEqual({ ok: true, destination: null })
    expect(checkDestination('   ')).toEqual({ ok: true, destination: null })
    expect(checkDestination('/')).toEqual({ ok: true, destination: null })
  })

  it('normalises an absolute directory', () => {
    expect(checkDestination(' /srv//restore/ ')).toEqual({ ok: true, destination: '/srv/restore' })
    expect(checkDestination('/srv/..old/.cache')).toEqual({
      ok: true,
      destination: '/srv/..old/.cache',
    })
  })

  it('refuses what the server and agent refuse', () => {
    expect(checkDestination('srv/restore').ok).toBe(false)
    expect(checkDestination('/srv/../etc').ok).toBe(false)
    expect(checkDestination('/srv/./restore').ok).toBe(false)
    expect(checkDestination('/srv/re\u0000store').ok).toBe(false)
    expect(checkDestination(`/${'a'.repeat(4096)}`).ok).toBe(false)
  })
})

describe('restoredLocation', () => {
  it('keeps the archive structure under the destination', () => {
    expect(restoredLocation('/srv/restore', 'etc/nginx/nginx.conf')).toBe(
      '/srv/restore/etc/nginx/nginx.conf',
    )
    expect(restoredLocation('/srv/restore', '')).toBe('/srv/restore')
  })

  it('puts a path back where it came from without a destination', () => {
    expect(restoredLocation(null, 'etc/hosts')).toBe('/etc/hosts')
    expect(restoredLocation(null, '/etc/hosts')).toBe('/etc/hosts')
  })
})
