// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import MachineEffectList from './MachineEffectList.vue'

describe('MachineEffectList', () => {
  it('shows each machine with its role, its effect and the choice behind it', () => {
    const wrapper = renderWithPlugins(MachineEffectList, {
      props: {
        rows: [
          {
            key: 1,
            name: 'nas.lan',
            role: 'repository host',
            effect: { label: 'Waits', tone: 'info', reason: 'Not woken.' },
            choice: 'Wait for it',
          },
          {
            key: 2,
            name: 'web-01',
            role: 'agent',
            effect: { label: 'Wakes', tone: 'success', reason: 'Woken.' },
          },
        ],
      },
    })

    const terms = wrapper.findAll('dt').map((d) => d.text())
    expect(terms).toEqual(['nas.lanrepository host', 'web-01agent'])
    const badges = wrapper.findAll('.badge')
    expect(badges.map((b) => b.text())).toEqual(['Waits', 'Wakes'])
    expect(badges[0]!.classes()).toContain('badge--info')
    expect(wrapper.findAll('dd')[0]!.text()).toContain('Wait for it')
    expect(wrapper.findAll('dd')[1]!.text()).not.toContain('Wait for it')
  })
})
