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

  it('defines a cache name', () => {
    expect(swContent).toContain('CACHE_NAME')
  })

  it('registers a fetch event listener that caches responses', () => {
    expect(swContent).toContain("addEventListener('fetch'")
    expect(swContent).toContain('caches.open')
    expect(swContent).toContain('cache.put')
  })

  it('serves from cache on network failure', () => {
    expect(swContent).toContain('caches.match')
  })

  it('skips API and WebSocket requests', () => {
    expect(swContent).toContain('/api/')
    expect(swContent).toContain('/ws/')
  })

  it('cleans up old caches on activate', () => {
    expect(swContent).toContain('caches.keys')
    expect(swContent).toContain('caches.delete')
  })
})
