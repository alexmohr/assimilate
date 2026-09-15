// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import PagerLoadMore from './PagerLoadMore.vue'

describe('PagerLoadMore', () => {
  it('hides the button once every item is loaded', () => {
    const wrapper = mount(PagerLoadMore, { props: { loaded: 2, total: 2, loadingMore: false } })
    expect(wrapper.find('button').exists()).toBe(false)
  })

  it('offers to load more, capped at a page, with the default label', () => {
    const wrapper = mount(PagerLoadMore, { props: { loaded: 2, total: 312, loadingMore: false } })
    expect(wrapper.find('button').text()).toBe('Load 50 more')
  })

  it('offers the exact remainder when fewer than a full page is left', () => {
    const wrapper = mount(PagerLoadMore, { props: { loaded: 2, total: 5, loadingMore: false } })
    expect(wrapper.find('button').text()).toBe('Load 3 more')
  })

  it('uses a custom load label when given one', () => {
    const wrapper = mount(PagerLoadMore, {
      props: { loaded: 2, total: 312, loadingMore: false, loadLabel: 'more runs' },
    })
    expect(wrapper.find('button').text()).toBe('Load 50 more runs')
  })

  it('emits loadMore when the button is clicked', async () => {
    const wrapper = mount(PagerLoadMore, { props: { loaded: 2, total: 312, loadingMore: false } })
    await wrapper.find('button').trigger('click')
    expect(wrapper.emitted('loadMore')).toHaveLength(1)
  })

  it('disables the button and shows a loading label while a page is in flight', () => {
    const wrapper = mount(PagerLoadMore, { props: { loaded: 2, total: 312, loadingMore: true } })
    const button = wrapper.find('button')
    expect(button.attributes('disabled')).toBeDefined()
    expect(button.text()).toBe('Loading...')
  })

  it('renders the slot content as the note', () => {
    const wrapper = mount(PagerLoadMore, {
      props: { loaded: 2, total: 312, loadingMore: false },
      slots: { default: 'Showing 2 of 312 runs' },
    })
    expect(wrapper.text()).toContain('Showing 2 of 312 runs')
  })
})
