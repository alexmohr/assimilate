// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { defineComponent, h } from 'vue'
import { flushPromises, mount } from '@vue/test-utils'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import {
  parseArchiveName,
  parseArchivePath,
  useArchivePathRoute,
  useArchiveSelectionRoute,
  type ArchivePathRoute,
  type ArchiveSelectionRoute,
} from './useArchivePathRoute'

/** A router over two pages, so a test can tell staying from leaving. */
function makeRouter(): Router {
  const Page = defineComponent({ render: () => h('div') })
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', component: Page },
      { path: '/repos/:id', component: Page },
    ],
  })
}

describe('parseArchivePath', () => {
  it('reads a missing or non-string value as the root', () => {
    expect(parseArchivePath(undefined)).toBe('/')
    expect(parseArchivePath(null)).toBe('/')
    expect(parseArchivePath(['/a', '/b'])).toBe('/')
    expect(parseArchivePath('')).toBe('/')
    expect(parseArchivePath('/')).toBe('/')
  })

  it('normalizes a directory to a leading slash and no trailing one', () => {
    expect(parseArchivePath('tmp')).toBe('/tmp')
    expect(parseArchivePath('/tmp/')).toBe('/tmp')
    expect(parseArchivePath('//tmp//./etc/')).toBe('/tmp/etc')
  })

  it('opens the root instead of climbing out with ..', () => {
    expect(parseArchivePath('/tmp/../etc')).toBe('/')
    expect(parseArchivePath('..')).toBe('/')
  })

  it('keeps names that only start with dots', () => {
    expect(parseArchivePath('/home/.config/..hidden')).toBe('/home/.config/..hidden')
  })
})

describe('useArchivePathRoute', () => {
  async function setup(
    router: Router | null,
    key = 'path',
  ): Promise<{ api: ArchivePathRoute; unmount: () => void }> {
    let api: ArchivePathRoute | null = null
    const Host = defineComponent({
      setup() {
        api = useArchivePathRoute(() => key)
        return (): ReturnType<typeof h> => h('div')
      },
    })
    const wrapper = mount(Host, { global: { plugins: router ? [router] : [] } })
    await flushPromises()
    return { api: api as unknown as ArchivePathRoute, unmount: (): void => wrapper.unmount() }
  }

  it('stays disabled at the root without a router', async () => {
    const { api } = await setup(null)
    expect(api.enabled).toBe(false)
    expect(api.path.value).toBe('/')
    // Nothing to push to; neither call may throw.
    api.push('/tmp')
    api.clear()
    api.release()
    expect(api.path.value).toBe('/')
  })

  it('reads the directory from its own query key', async () => {
    const router = makeRouter()
    await router.push('/repos/1?tab=archives&path=%2Ftmp&path-2=%2Fetc')
    const { api } = await setup(router, 'path-2')
    expect(api.enabled).toBe(true)
    expect(api.path.value).toBe('/etc')
  })

  it('pushes a history entry per directory and keeps other query params', async () => {
    const router = makeRouter()
    await router.push('/repos/1?tab=archives')
    const { api } = await setup(router)

    api.push('/tmp')
    await flushPromises()
    expect(router.currentRoute.value.query).toEqual({ tab: 'archives', path: '/tmp' })
    expect(api.path.value).toBe('/tmp')

    api.push('/')
    await flushPromises()
    // The root is the entry without the key, not one with an empty value.
    expect(router.currentRoute.value.query).toEqual({ tab: 'archives' })

    router.back()
    await flushPromises()
    expect(api.path.value).toBe('/tmp')
  })

  it('clear replaces the current entry instead of adding one', async () => {
    const router = makeRouter()
    await router.push('/')
    await router.push('/repos/1?tab=archives&path=%2Ftmp')
    const { api } = await setup(router)

    api.clear()
    await flushPromises()
    expect(router.currentRoute.value.fullPath).toBe('/repos/1?tab=archives')

    router.back()
    await flushPromises()
    expect(router.currentRoute.value.path).toBe('/')
  })

  it('release drops the directory only while the page it was mounted on stays', async () => {
    const router = makeRouter()
    await router.push('/repos/1?path=%2Ftmp')
    const first = await setup(router)
    first.api.release()
    await flushPromises()
    expect(router.currentRoute.value.query).toEqual({})

    await router.push('/repos/1?path=%2Ftmp')
    const second = await setup(router)
    await router.push('/?path=%2Fother')
    second.api.release()
    await flushPromises()
    expect(router.currentRoute.value.path).toBe('/')
    expect(router.currentRoute.value.query).toEqual({ path: '/other' })
  })
})

describe('parseArchiveName', () => {
  it('reads one non-empty string and nothing else', () => {
    expect(parseArchiveName('web-01-2026-03-01')).toBe('web-01-2026-03-01')
    expect(parseArchiveName('')).toBeNull()
    expect(parseArchiveName(undefined)).toBeNull()
    expect(parseArchiveName(null)).toBeNull()
    expect(parseArchiveName(['a', 'b'])).toBeNull()
  })
})

describe('useArchiveSelectionRoute', () => {
  async function setup(
    router: Router | null,
    nameKey = 'browse',
    pathKey = 'path',
  ): Promise<ArchiveSelectionRoute> {
    let api: ArchiveSelectionRoute | null = null
    const Host = defineComponent({
      setup() {
        api = useArchiveSelectionRoute(
          () => nameKey,
          () => pathKey,
        )
        return (): ReturnType<typeof h> => h('div')
      },
    })
    mount(Host, { global: { plugins: router ? [router] : [] } })
    await flushPromises()
    return api as unknown as ArchiveSelectionRoute
  }

  it('stays disabled with no archive without a router', async () => {
    const api = await setup(null)
    expect(api.enabled).toBe(false)
    expect(api.name.value).toBeNull()
    api.push('nightly')
    api.replace(null)
    api.release()
    expect(api.name.value).toBeNull()
  })

  it('reads the archive from its own query key', async () => {
    const router = makeRouter()
    await router.push('/repos/1?browse=a&browse-2=b')
    const api = await setup(router, 'browse-2', 'path-2')
    expect(api.enabled).toBe(true)
    expect(api.name.value).toBe('b')
  })

  it('pushes a history entry per archive, opening each at its root', async () => {
    const router = makeRouter()
    await router.push('/repos/1?tab=archives&browse=a&path=%2Fetc')
    const api = await setup(router)

    api.push('b')
    await flushPromises()
    // The directory belonged to the archive left behind.
    expect(router.currentRoute.value.query).toEqual({ tab: 'archives', browse: 'b' })

    router.back()
    await flushPromises()
    expect(router.currentRoute.value.query).toEqual({ tab: 'archives', browse: 'a', path: '/etc' })
    expect(api.name.value).toBe('a')
  })

  it('replace rewrites the current entry, archive and directory together', async () => {
    const router = makeRouter()
    await router.push('/')
    await router.push('/repos/1?tab=archives&browse=a&path=%2Fetc')
    const api = await setup(router)

    api.replace('b')
    await flushPromises()
    expect(router.currentRoute.value.fullPath).toBe('/repos/1?tab=archives&browse=b')

    api.replace(null)
    await flushPromises()
    expect(router.currentRoute.value.fullPath).toBe('/repos/1?tab=archives')

    router.back()
    await flushPromises()
    expect(router.currentRoute.value.path).toBe('/')
  })

  it('replace cancels a push that has not landed yet', async () => {
    const router = makeRouter()
    await router.push('/repos/1')
    const api = await setup(router)

    api.push('a')
    api.replace(null)
    await flushPromises()
    expect(router.currentRoute.value.query).toEqual({})
  })

  it('release drops archive and directory only while the page it was mounted on stays', async () => {
    const router = makeRouter()
    await router.push('/repos/1?browse=a&path=%2Fetc&tab=archives')
    const first = await setup(router)
    first.release()
    await flushPromises()
    expect(router.currentRoute.value.query).toEqual({ tab: 'archives' })

    await router.push('/repos/1?browse=a')
    const second = await setup(router)
    await router.push('/?browse=other')
    second.release()
    await flushPromises()
    expect(router.currentRoute.value.fullPath).toBe('/?browse=other')
  })
})
