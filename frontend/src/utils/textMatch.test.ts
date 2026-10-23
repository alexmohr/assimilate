// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { matchParts } from './textMatch'

describe('matchParts', () => {
  it('marks every occurrence, ignoring case and keeping the original text', () => {
    expect(matchParts('Nginx.conf.nginx', 'NGINX')).toEqual([
      { text: 'Nginx', match: true },
      { text: '.conf.', match: false },
      { text: 'nginx', match: true },
    ])
  })

  it('leaves the text whole when nothing matches or there is no needle', () => {
    expect(matchParts('hosts', 'conf')).toEqual([{ text: 'hosts', match: false }])
    expect(matchParts('hosts', '  ')).toEqual([{ text: 'hosts', match: false }])
  })

  it('marks a whole-text match without empty runs around it', () => {
    expect(matchParts('conf', 'conf')).toEqual([{ text: 'conf', match: true }])
  })
})
