// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { defineComponent, type ComponentPublicInstance } from 'vue'
import { mount, type VueWrapper } from '@vue/test-utils'
import {
  createRouter,
  createMemoryHistory,
  type RouteLocationRaw,
  type RouteRecordRaw,
} from 'vue-router'
import { createPinia, type Pinia } from 'pinia'
import { vi } from 'vitest'
import type { Component } from 'vue'
import { router as appRouter } from '../router'

export interface RenderWithPluginsOptions {
  props?: Record<string, unknown>
  slots?: Record<string, unknown>
  storeState?: Record<string, Record<string, unknown>>
  routeOverrides?: RouteLocationRaw
}

const routeStub = defineComponent({
  name: 'RouteStub',
  render: (): null => null,
})

function createTestingPinia(storeState: RenderWithPluginsOptions['storeState']): Pinia {
  const pinia = createPinia()

  pinia.use(({ store }) => {
    for (const key of Object.keys(store)) {
      const value = store[key as keyof typeof store]

      if (typeof value === 'function' && !key.startsWith('$')) {
        store[key as keyof typeof store] = vi.fn()
      }
    }

    const state = storeState?.[store.$id]
    if (state) {
      store.$patch(state)
    }
  })

  return pinia
}

function createRoutes(): RouteRecordRaw[] {
  return [
    {
      path: '/:pathMatch(.*)*',
      name: 'test-catch-all',
      component: routeStub,
    },
  ]
}

export function createMockRouter(): ReturnType<typeof createRouter> {
  const routes = appRouter.getRoutes().map((route) => ({
    path: route.path,
    name: route.name,
    component: routeStub,
    meta: route.meta,
    props: route.props,
    redirect: route.redirect,
  })) satisfies RouteRecordRaw[]

  return createRouter({
    history: createMemoryHistory(),
    routes,
  })
}

export function renderWithPlugins(
  component: Component,
  options: RenderWithPluginsOptions = {},
): VueWrapper<ComponentPublicInstance> {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: createRoutes(),
  })

  const pinia = createTestingPinia(options.storeState)

  void router.push(options.routeOverrides ?? '/')

  return mount(component, {
    props: options.props,
    slots: options.slots,
    global: {
      plugins: [pinia, router],
    },
  })
}
