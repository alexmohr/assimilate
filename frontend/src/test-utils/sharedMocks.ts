// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { vi } from 'vitest'
import {
  computed,
  defineComponent,
  h,
  inject,
  provide,
  type Component,
  type ComputedRef,
} from 'vue'

export function mockTimezone(): {
  useTimezone: ReturnType<typeof vi.fn>
  getConfiguredTimezone: ReturnType<typeof vi.fn>
} {
  return {
    useTimezone: vi.fn(),
    getConfiguredTimezone: vi.fn().mockReturnValue(undefined),
  }
}

export function mockApiClient(): { apiClient: { get: ReturnType<typeof vi.fn> } } {
  return { apiClient: { get: vi.fn() } }
}

/**
 * The client mocked across every verb, for views that write as well as read.
 * `mockApiClient` above stays read-only so a test that only lists something
 * cannot accidentally assert on a request it never makes.
 */
export function mockApiClientRw(): {
  apiClient: {
    get: ReturnType<typeof vi.fn>
    post: ReturnType<typeof vi.fn>
    put: ReturnType<typeof vi.fn>
    delete: ReturnType<typeof vi.fn>
  }
} {
  return { apiClient: { get: vi.fn(), post: vi.fn(), put: vi.fn(), delete: vi.fn() } }
}

export function mockFormatBytes(): { formatBytes: (bytes: number) => string } {
  return { formatBytes: (bytes: number): string => `${bytes} B` }
}

export function mockErrorUtils(): {
  extractError: (e: unknown) => string
  extractBlobError: (e: unknown) => Promise<string>
} {
  return {
    extractError: (_e: unknown): string => 'API error',
    extractBlobError: async (_e: unknown): Promise<string> => 'API error',
  }
}

/**
 * The same two extractors, passing the thrown message through. For a spec
 * that asserts the failure a component *shows*, rather than that it showed
 * one at all.
 */
export function mockErrorUtilsPassthrough(): {
  extractError: (e: unknown) => string
  extractBlobError: (e: unknown) => Promise<string>
} {
  const message = (e: unknown): string => (e instanceof Error ? e.message : 'Unknown error')
  return {
    extractError: message,
    extractBlobError: async (e: unknown): Promise<string> => message(e),
  }
}

/**
 * The toast composable, with the two spies a spec asserts on. Shared because
 * `vi.mock` hoists: every spec that wanted to check a toast declared the same
 * two `vi.fn()`s and the same factory above its imports. Reset them in
 * `beforeEach` - the object outlives a single test, though not the file.
 */
export const toastSpies = { success: vi.fn(), error: vi.fn() }

export function mockToast(): { useToast: () => typeof toastSpies } {
  return { useToast: () => toastSpies }
}

export function resetToastSpies(): void {
  toastSpies.success.mockReset()
  toastSpies.error.mockReset()
}

/**
 * The WebSocket composable, captured by message type rather than stubbed
 * silent - for a spec that needs to fire a `DataChanged` or similar event at
 * the component under test. `wsHandlers` is a singleton for the same reason
 * `toastSpies` is: `vi.mock` factories can't close over a per-test object.
 * Reset it in `beforeEach` with `resetWsHandlers`.
 */
export const wsHandlers: Record<string, (payload: unknown) => void> = {}

export function mockWebSocket(): {
  useWebSocket: () => { onMessage: (type: string, cb: (payload: unknown) => void) => void }
} {
  return {
    useWebSocket: () => ({
      onMessage: (type: string, cb: (payload: unknown) => void): void => {
        wsHandlers[type] = cb
      },
    }),
  }
}

export function resetWsHandlers(): void {
  for (const key of Object.keys(wsHandlers)) delete wsHandlers[key]
}

/** The read/write client, without the delete verb some specs never use. */
export function mockApiClientRead(): {
  apiClient: { get: ReturnType<typeof vi.fn>; post: ReturnType<typeof vi.fn> }
} {
  return { apiClient: { get: vi.fn(), post: vi.fn() } }
}

/**
 * `DataTable` / `Column` stubs that render each column's `#body` slot once per
 * row, so cell renderers (badge classes, formatters, links) are exercised.
 *
 * The usual `Column: true` stub drops those slots entirely, which leaves every
 * cell renderer in a table view unreachable from its tests.
 */
export function dataTableStubs(): {
  DataTable: Component
  Column: Component
} {
  const rowsKey = Symbol.for('assimilate.test.tableRows')

  const DataTable = defineComponent({
    name: 'DataTableStub',
    props: { value: { type: Array as () => unknown[] } },
    setup(props, { slots }) {
      provide(
        rowsKey,
        computed(() => props.value ?? []),
      )
      return (): ReturnType<typeof h> =>
        h('div', { class: 'p-datatable' }, [slots.default?.(), slots.empty?.()])
    },
  })

  const Column = defineComponent({
    name: 'ColumnStub',
    props: { header: { type: String, default: '' }, field: { type: String, default: '' } },
    setup(props, { slots }) {
      const rows = inject<ComputedRef<unknown[]> | undefined>(rowsKey, undefined)
      return (): ReturnType<typeof h> =>
        h(
          'div',
          { class: 'p-column', 'data-field': props.field },
          (rows?.value ?? []).map((row, i) =>
            h('div', { class: 'p-cell', key: i }, slots.body?.({ data: row })),
          ),
        )
    },
  })

  return { DataTable, Column }
}
