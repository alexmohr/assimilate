// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const swContent = readFileSync(resolve(__dirname, '../public/sw.js'), 'utf-8')

describe('service worker (sw.js)', () => {
  it('registers a push event listener', () => {
    expect(swContent).toContain("addEventListener('push'")
  })

  it('registers a notificationclick event listener', () => {
    expect(swContent).toContain("addEventListener('notificationclick'")
  })

  it('calls showNotification in the push handler', () => {
    expect(swContent).toContain('showNotification')
  })

  it('registers install and activate lifecycle handlers', () => {
    expect(swContent).toContain("addEventListener('install'")
    expect(swContent).toContain("addEventListener('activate'")
  })
})
