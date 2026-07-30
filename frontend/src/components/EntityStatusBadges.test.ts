// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import EntityStatusBadges from './EntityStatusBadges.vue'

describe('EntityStatusBadges', () => {
  it('renders nothing when there are no issues and the entity is not notable', () => {
    const wrapper = mount(EntityStatusBadges)
    expect(wrapper.find('.entity-badge-row').exists()).toBe(false)
  })

  it('renders an issue chip for each issue with its severity class', () => {
    const wrapper = mount(EntityStatusBadges, {
      props: {
        issues: [
          { key: 'failed', label: '1 failed', severity: 'danger', onClick: vi.fn() },
          { key: 'overdue', label: '2 overdue', severity: 'warning', onClick: vi.fn() },
        ],
      },
    })
    const chips = wrapper.findAll('.entity-issue-chip')
    expect(chips).toHaveLength(2)
    expect(chips[0].classes()).toContain('sev-danger')
    expect(chips[0].text()).toBe('1 failed')
    expect(chips[1].classes()).toContain('sev-warning')
    expect(chips[1].text()).toBe('2 overdue')
  })

  it('calls onClick and stops the click from bubbling to an ancestor', async () => {
    const onClick = vi.fn()
    const parentClick = vi.fn()
    const wrapper = mount(EntityStatusBadges, {
      props: {
        issues: [{ key: 'failed', label: 'Failed', severity: 'danger', onClick }],
      },
    })
    wrapper.element.addEventListener('click', parentClick)

    await wrapper.find('.entity-issue-chip').trigger('click')

    expect(onClick).toHaveBeenCalledOnce()
    expect(parentClick).not.toHaveBeenCalled()
  })

  it('sets the title attribute for a hover tooltip when provided', () => {
    const wrapper = mount(EntityStatusBadges, {
      props: {
        issues: [
          {
            key: 'overdue',
            label: '2 overdue',
            severity: 'warning',
            onClick: vi.fn(),
            title: 'bell — last backup: never\ngremlin — last backup: 21 Jul, 01:00',
          },
        ],
      },
    })
    expect(wrapper.find('.entity-issue-chip').attributes('title')).toBe(
      'bell — last backup: never\ngremlin — last backup: 21 Jul, 01:00',
    )
  })

  it('shows the notable pill with its label when notable is true', () => {
    const wrapper = mount(EntityStatusBadges, {
      props: { notable: true, notableLabel: 'Disabled' },
    })
    const pill = wrapper.find('.entity-status-pill')
    expect(pill.exists()).toBe(true)
    expect(pill.text()).toBe('Disabled')
  })

  it('does not show a pill when notable is false, even with issues present', () => {
    const wrapper = mount(EntityStatusBadges, {
      props: {
        issues: [{ key: 'failed', label: 'Failed', severity: 'danger', onClick: vi.fn() }],
      },
    })
    expect(wrapper.find('.entity-status-pill').exists()).toBe(false)
    expect(wrapper.find('.entity-issue-chip').exists()).toBe(true)
  })

  it('renders both issue chips and the notable pill together', () => {
    const wrapper = mount(EntityStatusBadges, {
      props: {
        notable: true,
        notableLabel: 'Offline',
        issues: [{ key: 'failed', label: 'Failed', severity: 'danger', onClick: vi.fn() }],
      },
    })
    expect(wrapper.find('.entity-issue-chip').exists()).toBe(true)
    expect(wrapper.find('.entity-status-pill').text()).toBe('Offline')
  })

  it('shows the running pill with its label when running is true', () => {
    const wrapper = mount(EntityStatusBadges, {
      props: { running: true, runningLabel: 'Backing up: server-daily' },
    })
    const pill = wrapper.find('.entity-running-pill')
    expect(pill.exists()).toBe(true)
    expect(pill.text()).toBe('Backing up: server-daily')
  })

  it('defaults the running pill label to "Running" when none is given', () => {
    const wrapper = mount(EntityStatusBadges, {
      props: { running: true },
    })
    expect(wrapper.find('.entity-running-pill').text()).toBe('Running')
  })

  it('renders the row for a running entity even without issues or notable', () => {
    const wrapper = mount(EntityStatusBadges, {
      props: { running: true },
    })
    expect(wrapper.find('.entity-badge-row').exists()).toBe(true)
  })

  it('does not show the running pill when running is false', () => {
    const wrapper = mount(EntityStatusBadges, {
      props: { running: false, notable: true, notableLabel: 'Offline' },
    })
    expect(wrapper.find('.entity-running-pill').exists()).toBe(false)
  })
})
