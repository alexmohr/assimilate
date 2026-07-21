// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import vm from 'node:vm'
import { describe, expect, it, vi } from 'vitest'

const swContent = readFileSync(resolve(__dirname, '../public/sw.js'), 'utf-8')

interface PushEventLike {
  data: { json: () => unknown } | null
  waitUntil: (p: Promise<unknown>) => void
}

/**
 * Actually executes sw.js in a minimal service-worker-shaped VM context and
 * returns its registered listeners plus a spy on `showNotification`. Unlike
 * asserting on the raw source text, this runs the real handler code against
 * a synthetic event so a broken payload access or wrong argument would fail
 * the test.
 */
function loadServiceWorker(): {
  push: (event: PushEventLike) => Promise<void>
  showNotification: ReturnType<typeof vi.fn>
} {
  const listeners: Record<string, (event: PushEventLike) => void> = {}
  const showNotification = vi.fn().mockResolvedValue(undefined)
  const selfObj = {
    addEventListener: (type: string, handler: (event: PushEventLike) => void) => {
      listeners[type] = handler
    },
    registration: { showNotification },
    skipWaiting: vi.fn(),
    clients: { claim: vi.fn(), matchAll: vi.fn().mockResolvedValue([]) },
    location: { origin: 'https://assimilate.example' },
  }
  const context = vm.createContext({
    self: selfObj,
    caches: { keys: vi.fn().mockResolvedValue([]) },
  })
  vm.runInContext(swContent, context)

  return {
    push: async (event: PushEventLike) => {
      const waits: Promise<unknown>[] = []
      listeners.push({ ...event, waitUntil: (p) => waits.push(p) })
      await Promise.all(waits)
    },
    showNotification,
  }
}

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
    expect(swContent).toContain('.keys()')
    expect(swContent).toContain('caches.delete')
  })
})

describe('service worker (sw.js) push handler behavior', () => {
  it('shows a notification built from the push payload', async () => {
    const sw = loadServiceWorker()
    await sw.push({
      data: {
        json: () => ({
          title: 'Backup Failed',
          body: 'host1 - failed: disk full',
          tag: 'backup_failed',
          url: '/agents/host1?tab=backups',
        }),
      },
      waitUntil: () => undefined,
    })

    expect(sw.showNotification).toHaveBeenCalledExactlyOnceWith('Backup Failed', {
      body: 'host1 - failed: disk full',
      icon: '/icon.png',
      badge: '/icon.png',
      tag: 'backup_failed',
      data: { url: '/agents/host1?tab=backups' },
    })
  })

  it('falls back to defaults when the push carries no data', async () => {
    const sw = loadServiceWorker()
    await sw.push({ data: null, waitUntil: () => undefined })

    expect(sw.showNotification).toHaveBeenCalledExactlyOnceWith('Assimilate', {
      body: '',
      icon: '/icon.png',
      badge: '/icon.png',
      tag: 'assimilate-notification',
      data: { url: '/' },
    })
  })
})
