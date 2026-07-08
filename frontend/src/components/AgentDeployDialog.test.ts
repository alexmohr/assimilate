// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import type { ComponentPublicInstance } from 'vue'
import AgentDeployDialog from './AgentDeployDialog.vue'

const postMock = vi.fn()

vi.mock('../api/client', () => ({
  apiClient: {
    post: (...args: unknown[]): unknown => postMock(...args),
  },
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown): string => 'API error',
}))

let wrapper: VueWrapper<ComponentPublicInstance> | null = null

function mountDialog(props: {
  hostname: string
  agentVersion: string | null
  lastSshUser?: string | null
}): VueWrapper<ComponentPublicInstance> {
  wrapper = mount(AgentDeployDialog, {
    props,
    attachTo: document.body,
  })
  return wrapper
}

describe('AgentDeployDialog', () => {
  beforeEach(() => {
    postMock.mockReset()
    postMock.mockResolvedValue({ data: { content: null } })
  })

  afterEach(() => {
    wrapper?.unmount()
    wrapper = null
  })

  it('defaults SSH user to root when no last-used username is known', async () => {
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()
    const sshUserInput = document.querySelector<HTMLInputElement>('input[placeholder="root"]')
    expect(sshUserInput?.value).toBe('root')
  })

  it('prefills SSH user from the lastSshUser prop', async () => {
    mountDialog({
      hostname: 'web-server-01',
      agentVersion: '1.0.0',
      lastSshUser: 'deploy-user',
    })
    await flushPromises()
    const sshUserInput = document.querySelector<HTMLInputElement>('input[placeholder="root"]')
    expect(sshUserInput?.value).toBe('deploy-user')
  })

  it('automatically loads the existing service unit from the remote host on mount', async () => {
    postMock.mockResolvedValue({
      data: { content: '[Service]\nEnvironment=BORG_AGENT_TOKEN=[REDACTED]\n' },
    })
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()
    expect(postMock).toHaveBeenCalledWith(
      '/agents/web-server-01/service-unit',
      expect.objectContaining({ ssh_host: 'web-server-01' }),
    )
    const textarea = document.querySelector<HTMLTextAreaElement>('textarea')
    expect(textarea?.value).toContain('Environment=BORG_AGENT_TOKEN=[REDACTED]')
  })

  it('does not surface an error banner when the automatic load finds no remote unit', async () => {
    postMock.mockResolvedValue({ data: { content: null } })
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()
    expect(document.querySelector('.field-hint-error')).toBeNull()
  })

  it('does not surface an error banner when the automatic load fails to connect', async () => {
    postMock.mockRejectedValue(new Error('connection refused'))
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()
    expect(document.querySelector('.field-hint-error')).toBeNull()
  })

  it('shows an error banner when a manual "Load from remote" click finds no unit', async () => {
    postMock.mockResolvedValue({ data: { content: null } })
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()
    const loadBtn = Array.from(document.querySelectorAll<HTMLButtonElement>('button')).find((b) =>
      b.textContent?.includes('Load from remote'),
    )
    loadBtn?.click()
    await flushPromises()
    expect(document.querySelector('.field-hint-error')?.textContent).toContain(
      'No existing service unit found',
    )
  })

  it('does not clobber in-progress edits with the result of the automatic load', async () => {
    let resolvePost: (value: { data: { content: string | null } }) => void = () => {}
    postMock.mockReturnValue(
      new Promise((resolve) => {
        resolvePost = resolve
      }),
    )
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()

    const textarea = document.querySelector<HTMLTextAreaElement>('textarea')
    expect(textarea).not.toBeNull()
    textarea!.value = 'user in-progress edit'
    textarea!.dispatchEvent(new Event('input'))
    await flushPromises()

    resolvePost({ data: { content: '[Service]\nEnvironment=BORG_AGENT_TOKEN=[REDACTED]\n' } })
    await flushPromises()

    expect(textarea?.value).toBe('user in-progress edit')
  })

  it('emits close when Cancel is clicked', async () => {
    const w = mountDialog({ hostname: 'web-server-01', agentVersion: null })
    await flushPromises()
    const cancelBtn = Array.from(document.querySelectorAll<HTMLButtonElement>('button')).find(
      (b) => b.textContent?.trim() === 'Cancel',
    )
    cancelBtn?.click()
    await w.vm.$nextTick()
    expect(w.emitted('close')).toBeTruthy()
  })
})
