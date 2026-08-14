// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import EntityCard from './EntityCard.vue'

describe('EntityCard', () => {
  it('renders the title and omits the subtitle when none is given', () => {
    const wrapper = mount(EntityCard, { props: { title: 'bell' } })
    expect(wrapper.find('.entity-card-title').text()).toBe('bell')
    expect(wrapper.find('.entity-card-subtitle').exists()).toBe(false)
  })

  it('renders the subtitle when provided', () => {
    const wrapper = mount(EntityCard, { props: { title: 'bell', subtitle: 'bell.local' } })
    expect(wrapper.find('.entity-card-subtitle').text()).toBe('bell.local')
  })

  it('renders stat entries in order with mono styling applied per-entry', () => {
    const wrapper = mount(EntityCard, {
      props: {
        title: 'bell',
        stats: [
          { value: 3, label: 'Schedules' },
          { value: '0.1.89', label: 'Agent', mono: true },
        ],
      },
    })
    const stats = wrapper.findAll('.entity-card-stat')
    expect(stats).toHaveLength(2)
    expect(stats[0].find('.entity-card-stat-value').text()).toBe('3')
    expect(stats[0].find('.entity-card-stat-value').classes()).not.toContain('mono')
    expect(stats[1].find('.entity-card-stat-label').text()).toBe('Agent')
    expect(stats[1].find('.entity-card-stat-value').classes()).toContain('mono')
  })

  it('omits the stats row entirely when no stats are given', () => {
    const wrapper = mount(EntityCard, { props: { title: 'bell' } })
    expect(wrapper.find('.entity-card-stats').exists()).toBe(false)
  })

  it('only renders optional regions whose slot content is actually provided', () => {
    const wrapper = mount(EntityCard, { props: { title: 'bell' } })
    expect(wrapper.find('.entity-card-top-badges').exists()).toBe(false)
    expect(wrapper.find('.entity-card-meta').exists()).toBe(false)
    expect(wrapper.find('.entity-card-actions').exists()).toBe(false)
  })

  it('renders slot content for top-badges, meta, status and actions', () => {
    const wrapper = mount(EntityCard, {
      props: { title: 'bell' },
      slots: {
        'top-badges': '<span class="my-badge">Hidden</span>',
        status: '<span class="my-status">Offline</span>',
        meta: '<span class="my-tag">prod</span>',
        actions: '<button class="my-action">Deploy</button>',
      },
    })
    expect(wrapper.find('.entity-card-top-badges .my-badge').text()).toBe('Hidden')
    expect(wrapper.find('.my-status').text()).toBe('Offline')
    expect(wrapper.find('.entity-card-meta .my-tag').text()).toBe('prod')
    expect(wrapper.find('.entity-card-actions .my-action').text()).toBe('Deploy')
  })

  it('stops action-slot clicks from bubbling to the card, but allows other clicks through', async () => {
    const cardClick = vi.fn()
    const actionClick = vi.fn()
    const wrapper = mount(EntityCard, {
      props: { title: 'bell' },
      attrs: { onClick: cardClick },
      slots: {
        actions: '<button class="my-action">Deploy</button>',
      },
    })
    wrapper.find('.my-action').element.addEventListener('click', actionClick)

    await wrapper.find('.my-action').trigger('click')
    expect(actionClick).toHaveBeenCalledOnce()
    expect(cardClick).not.toHaveBeenCalled()

    await wrapper.find('.entity-card-title').trigger('click')
    expect(cardClick).toHaveBeenCalledOnce()
  })
})
