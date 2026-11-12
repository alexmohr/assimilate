// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import PrimeVueConfig from 'primevue/config'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import Select from 'primevue/select'
import Paginator from 'primevue/paginator'
import { globalPrimeVuePT } from './primevue-pt'

describe('globalPrimeVuePT datatable.column', () => {
  it('gives sortable header cells a pointer cursor and lays out title+icon in a row', () => {
    const wrapper = mount(
      {
        components: { DataTable, Column },
        template: `
          <DataTable :value="[{ name: 'a' }]">
            <Column field="name" header="Name" :sortable="true" />
            <Column field="size" header="Size" />
          </DataTable>
        `,
      },
      {
        global: {
          plugins: [[PrimeVueConfig, { unstyled: true, pt: globalPrimeVuePT }]],
        },
      },
    )

    const headers = wrapper.findAll('th')
    const sortableHeader = headers[0]
    const plainHeader = headers[1]

    // Sortable column: clickable affordance on the header cell.
    expect(sortableHeader.classes()).toContain('cursor-pointer')
    // Plain column: no bogus pointer cursor since it isn't sortable.
    expect(plainHeader.classes()).not.toContain('cursor-pointer')

    // Without an explicit flex row, Tailwind's `svg { display: block }`
    // preflight knocks the sort icon onto its own line below the title.
    const content = sortableHeader.find('[data-pc-section="columnheadercontent"]')
    expect(content.classes()).toContain('flex')
  })
})

describe('globalPrimeVuePT select and paginator', () => {
  const global = { plugins: [[PrimeVueConfig, { unstyled: true, pt: globalPrimeVuePT }]] }

  it('styles a Select root as the app input box', () => {
    const wrapper = mount(Select, {
      props: { options: ['a', 'b'], modelValue: 'a' },
      global: global as never,
    })
    const root = wrapper.find('[data-pc-name="select"]')
    expect(root.classes()).toContain('bg-[var(--bg-input)]')
    expect(root.classes()).toContain('w-full')
  })

  it('styles paginator page buttons as bordered controls', () => {
    const wrapper = mount(Paginator, {
      props: { rows: 10, totalRecords: 30, first: 10 },
      global: global as never,
    })
    const pages = wrapper.findAll('[data-pc-section="page"]')
    expect(pages).toHaveLength(3)
    for (const page of pages) {
      expect(page.classes()).toContain('min-w-8')
      expect(page.classes()).toContain('border-[var(--border)]')
    }
    const prev = wrapper.find('[data-pc-section="prev"]')
    expect(prev.classes()).toContain('disabled:opacity-50')
  })
})
