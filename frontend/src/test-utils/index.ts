// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { defineComponent, h, type ComponentPublicInstance } from 'vue'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import {
  createRouter,
  createMemoryHistory,
  type RouteLocationRaw,
  type RouteRecordRaw,
} from 'vue-router'
import { createPinia, type Pinia } from 'pinia'
import { vi, expect } from 'vitest'
import type { Component } from 'vue'
import { router as appRouter } from '../router'
import BaseModal from '../components/BaseModal.vue'

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

const routerLinkStub = defineComponent({
  name: 'RouterLinkStub',
  props: {
    to: {
      type: [String, Object],
      default: null,
    },
  },
  setup(props, { slots }) {
    function resolveHref(): string {
      if (typeof props.to === 'string') return props.to
      const dest = props.to as { path?: string; query?: Record<string, string> }
      const path = dest.path ?? ''
      if (!dest.query || Object.keys(dest.query).length === 0) return path
      return `${path}?${new URLSearchParams(dest.query).toString()}`
    }
    return (): ReturnType<typeof h> => h('a', { href: resolveHref() }, slots.default?.())
  },
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
      store.$patch(state as Record<string, unknown> & Record<never, never>)
    }
  })

  return pinia
}

function createRoutes(): RouteRecordRaw[] {
  return [
    ...appRouter.getRoutes().map(
      (route) =>
        ({
          path: route.path,
          name: route.name,
          component: routeStub,
          meta: route.meta,
        }) as RouteRecordRaw,
    ),
    {
      path: '/:pathMatch(.*)*',
      name: 'test-catch-all',
      component: routeStub,
    },
  ]
}

export function createMockRouter(): ReturnType<typeof createRouter> {
  const routes = appRouter.getRoutes().map(
    (route) =>
      ({
        path: route.path,
        name: route.name,
        component: routeStub,
        meta: route.meta,
      }) as RouteRecordRaw,
  )

  return createRouter({
    history: createMemoryHistory(),
    routes,
  })
}

/**
 * Finds the control inside the `.field` whose `.field-label` contains `label`.
 *
 * Forms across this app share the `.field` / `.field-label` shape, so tests
 * address inputs by the label the user actually reads rather than by an index
 * or a CSS hook that a template edit would silently move.
 */
export function fieldByLabel(
  wrapper: VueWrapper<ComponentPublicInstance>,
  label: string,
): ReturnType<VueWrapper<ComponentPublicInstance>['find']> {
  const field = wrapper
    .findAll('.field')
    .find((f) => f.find('.field-label').exists() && f.find('.field-label').text().includes(label))
  const control = field?.find('input, select, textarea')
  if (!control || !control.exists()) throw new Error(`no field labelled "${label}"`)
  return control
}

/** Sets the value of the control found by {@link fieldByLabel}. */
export async function setFieldByLabel(
  wrapper: VueWrapper<ComponentPublicInstance>,
  label: string,
  value: string,
): Promise<void> {
  await fieldByLabel(wrapper, label).setValue(value)
}

/** Finds a `<button>` by its visible text and clicks it - shared by tests that open a modal via a toolbar action button. */
export async function clickButtonWithText(
  wrapper: VueWrapper<ComponentPublicInstance>,
  text: string,
): Promise<void> {
  const button = wrapper.findAll('button').find((b) => b.text().includes(text))
  if (!button) throw new Error(`No button found with text containing "${text}"`)
  await button.trigger('click')
}

/**
 * Drives the common "row action opens a ConfirmDeleteDialog" flow: opens it via the row's
 * first `.btn-danger-text` button, dismisses it via the close button and asserts the delete
 * API was not called, then reopens and confirms, asserting the delete API was called with
 * `expectedArg`. Shared by list views (Groups, Roles, Tokens, ...) whose delete confirmation
 * is otherwise identical apart from the API call being asserted.
 */
export async function cancelThenConfirmDelete(
  wrapper: VueWrapper<ComponentPublicInstance>,
  mockDelete: ReturnType<typeof vi.fn>,
  expectedArg: string,
): Promise<void> {
  const deleteButton = wrapper.findAll('button.btn-danger-text')[0]
  await deleteButton!.trigger('click')

  await wrapper.find('button.modal-close').trigger('click')
  await flushPromises()
  expect(wrapper.find('.modal-backdrop').exists()).toBe(false)
  expect(mockDelete).not.toHaveBeenCalled()

  await deleteButton!.trigger('click')
  await wrapper.find('button.btn-danger').trigger('click')
  await flushPromises()
  expect(mockDelete).toHaveBeenCalledWith(expectedArg)
}

/**
 * Every `BaseModal` currently open.
 *
 * Views declare all the dialogs they own side by side and switch them with
 * `:open`, so addressing "the open one" is stable where an index into
 * `findAllComponents` moves whenever a dialog is added.
 */
export function openModals(
  wrapper: VueWrapper<ComponentPublicInstance>,
): VueWrapper<ComponentPublicInstance>[] {
  return wrapper
    .findAllComponents(BaseModal)
    .filter((m) => m.props('open') === true) as unknown as VueWrapper<ComponentPublicInstance>[]
}

/** The single open modal; throws if the view has none or several. */
export function openModal(
  wrapper: VueWrapper<ComponentPublicInstance>,
): VueWrapper<ComponentPublicInstance> {
  const open = openModals(wrapper)
  if (open.length !== 1) throw new Error(`expected exactly one open modal, found ${open.length}`)
  return open[0]
}

/**
 * Dismisses the open modal the way Escape and a backdrop click do.
 *
 * That path is wired separately from the dialog's own Cancel button, so a
 * dialog can close cleanly one way and leak state the other.
 */
export async function dismissModal(wrapper: VueWrapper<ComponentPublicInstance>): Promise<void> {
  openModal(wrapper).vm.$emit('close')
  await flushPromises()
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
    // Attached so `document`-based queries see the markup; combined with the
    // Teleport stub below, modal content is reachable both ways.
    attachTo: document.body,
    props: options.props,
    slots: options.slots,
    global: {
      plugins: [pinia, router],
      stubs: {
        RouterLink: routerLinkStub,
        // Modals render through BaseModal, which teleports to <body>. Without
        // this, their content lands outside the wrapper and `wrapper.find`
        // cannot reach it. Matches what the component tests already do.
        Teleport: true,
      },
    },
  })
}
