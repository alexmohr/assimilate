// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { defineComponent, h } from 'vue'
import { flushPromises, mount } from '@vue/test-utils'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import { parseArchivePath, useArchivePathRoute, type ArchivePathRoute } from './useArchivePathRoute'

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
