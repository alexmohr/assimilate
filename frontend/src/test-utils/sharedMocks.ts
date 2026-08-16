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
