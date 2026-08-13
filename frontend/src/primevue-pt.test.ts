// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import PrimeVueConfig from 'primevue/config'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
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
