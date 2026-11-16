// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { PROVIDER_LABELS } from './hostProvider'

describe('PROVIDER_LABELS', () => {
  it('names every provider the way the docs do', () => {
    expect(PROVIDER_LABELS).toEqual({
      generic: 'Generic SSH',
      hetzner: 'Hetzner Storage Box',
      rsync_net: 'rsync.net',
      borgbase: 'BorgBase',
    })
  })
})
