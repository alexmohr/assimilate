// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, type MockInstance } from 'vitest'
import { mount } from '@vue/test-utils'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
  },
}))

vi.mock('../utils/error', () => ({
  extractError: (e: unknown): string => {
    if (e instanceof Error) return e.message
    return 'Unknown error'
  },
}))

import { apiClient } from '../api/client'
import RestoreWizard from './RestoreWizard.vue'

const mockPost = apiClient.post as MockInstance

const ARCHIVES = [
  {
    name: 'web-server-01-2026-05-30T12:00:00',
    start: '2026-05-30T12:00:00',
    hostname: 'web-server-01',
    comment: 'pre-upgrade',
  },
  {
    name: 'web-server-01-2026-05-29T12:00:00',
    start: '2026-05-29T12:00:00',
    hostname: 'web-server-01',
    comment: 'weekly-baseline',
  },
]

function mountWizard(open = true): ReturnType<typeof mount> {
  return mount(RestoreWizard, {
    props: { open, repoId: 1, archives: ARCHIVES },
    attachTo: document.body,
    global: {
      stubs: {
        BaseModal: {
          template: '<div v-if="open" class="stub-modal"><slot /><slot name="footer" /></div>',
          props: ['open', 'title', 'size'],
        },
        Teleport: true,
      },
    },
  })
}

describe('RestoreWizard', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('does not render content when open is false', () => {
    const wrapper = mountWizard(false)
    expect(wrapper.find('.stub-modal').exists()).toBe(false)
  })

  it('renders step indicators for all 4 steps', () => {
    const wrapper = mountWizard()
    const dots = wrapper.findAll('.step-dot')
    expect(dots).toHaveLength(4)
  })

  it('shows step 1 content with archive selector', () => {
    const wrapper = mountWizard()
    expect(wrapper.find('.step-content select').exists()).toBe(true)
    expect(wrapper.text()).toContain('Select Archive')
  })

  it('renders all archive options in step 1', () => {
    const wrapper = mountWizard()
    const options = wrapper
      .findAll('.step-content select option')
      .map((o) => o.text())
      .filter((t) => t.includes('web-server'))
    expect(options).toHaveLength(2)
    expect(options[0]).toContain('web-server-01-2026-05-30T12:00:00')
  })

  it('disables Next button on step 1 until an archive is selected', async () => {
    const wrapper = mountWizard()
    const nextBtn = wrapper.findAll('button').find((b) => b.text() === 'Next')
    expect(nextBtn).toBeDefined()
    expect(nextBtn!.attributes('disabled')).toBeDefined()
  })

  it('enables Next button after selecting an archive', async () => {
    const wrapper = mountWizard()
    await wrapper.find('.step-content select').setValue(ARCHIVES[0].name)
    const nextBtn = wrapper.findAll('button').find((b) => b.text() === 'Next')
    expect(nextBtn!.attributes('disabled')).toBeUndefined()
  })

  it('advances to step 2 after selecting archive and clicking Next', async () => {
    const wrapper = mountWizard()
    await wrapper.find('.step-content select').setValue(ARCHIVES[0].name)
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('Paths to restore')
    expect(wrapper.find('textarea').exists()).toBe(true)
  })

  it('shows Back button on step 2 and navigates back to step 1', async () => {
    const wrapper = mountWizard()
    await wrapper.find('.step-content select').setValue(ARCHIVES[0].name)
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    const backBtn = wrapper.findAll('button').find((b) => b.text() === 'Back')
    expect(backBtn).toBeDefined()
    await backBtn!.trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('Select Archive')
  })

  it('disables Next on step 2 until paths are entered', async () => {
    const wrapper = mountWizard()
    await wrapper.find('.step-content select').setValue(ARCHIVES[0].name)
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    const nextBtn = wrapper.findAll('button').find((b) => b.text() === 'Next')
    expect(nextBtn!.attributes('disabled')).toBeDefined()
  })

  it('advances through all steps and reaches step 4 confirmation', async () => {
    const wrapper = mountWizard()

    await wrapper.find('.step-content select').setValue(ARCHIVES[0].name)
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper.find('textarea').setValue('/etc/nginx/nginx.conf')
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('Confirm Restore')
    expect(wrapper.text()).toContain(ARCHIVES[0].name)
    expect(wrapper.text()).toContain('/etc/nginx/nginx.conf')
    expect(wrapper.find('button.btn-primary').text()).toBe('Restore')
  })

  it('calls download API and shows success on step 4 execute', async () => {
    mockPost.mockResolvedValue({
      data: new Blob(['tar-content'], { type: 'application/x-tar' }),
    })

    global.URL.createObjectURL = vi.fn().mockReturnValue('blob:test')
    global.URL.revokeObjectURL = vi.fn()

    const wrapper = mountWizard()

    await wrapper.find('.step-content select').setValue(ARCHIVES[0].name)
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper.find('textarea').setValue('/etc/nginx/nginx.conf')
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper.find('button.btn-primary').trigger('click')
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    expect(mockPost).toHaveBeenCalledWith(
      expect.stringContaining('/download'),
      expect.objectContaining({ paths: ['/etc/nginx/nginx.conf'] }),
      expect.objectContaining({ responseType: 'blob' }),
    )
    expect(wrapper.text()).toContain('Restore completed successfully.')
  })

  it('shows error on step 4 when API fails', async () => {
    mockPost.mockRejectedValue(new Error('Restore failed'))

    const wrapper = mountWizard()

    await wrapper.find('.step-content select').setValue(ARCHIVES[0].name)
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper.find('textarea').setValue('/etc/nginx/nginx.conf')
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Next')!
      .trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper.find('button.btn-primary').trigger('click')
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    expect(wrapper.find('.form-error').text()).toBe('Restore failed')
  })
})
