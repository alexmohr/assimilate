// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import type { ComponentPublicInstance } from 'vue'
import AgentDeployDialog from './AgentDeployDialog.vue'
import BaseModal from './BaseModal.vue'

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
  availableVersion?: string | null
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

  describe('version summary', () => {
    it('shows the installed and available versions and names the target on the submit button', () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0', availableVersion: '1.2.0' })
      expect(document.querySelector('.upgrade-hero')?.textContent).toContain('1.0.0')
      expect(document.querySelector('.upgrade-hero')?.textContent).toContain('1.2.0')
      const submitBtn = Array.from(document.querySelectorAll<HTMLButtonElement>('button')).find(
        (b) => b.textContent?.includes('Upgrade to'),
      )
      expect(submitBtn?.textContent?.trim()).toBe('Upgrade to 1.2.0')
    })

    it('falls back to a generic label when the two version strings are identical', () => {
      // A dev build can be newer without its semantic version changing (the
      // server compares by git commit count instead).
      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0', availableVersion: '1.0.0' })
      expect(document.querySelector('.upgrade-hero')?.textContent).toContain(
        'A newer build is available.',
      )
      const submitBtn = Array.from(document.querySelectorAll<HTMLButtonElement>('button')).find(
        (b) => b.textContent?.trim() === 'Upgrade Agent',
      )
      expect(submitBtn).toBeDefined()
    })

    it('shows no version hero for a first-time deploy', () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: null })
      expect(document.querySelector('.upgrade-hero')).toBeNull()
    })
  })

  describe('service unit disclosure', () => {
    function isOpen(): boolean {
      const body = document.querySelector<HTMLElement>('.disclosure-body')
      return body !== null && body.style.display !== 'none'
    }

    function toggle(): void {
      document.querySelector<HTMLButtonElement>('.disclosure-head')?.click()
    }

    it('starts collapsed on an upgrade, where a working unit already exists', async () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
      await flushPromises()
      expect(isOpen()).toBe(false)
    })

    // A first-time deploy has no established unit yet, so loading or editing
    // one is part of setting the host up rather than a detail to scroll past.
    it('starts open on a first-time deploy', async () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: null })
      await flushPromises()
      expect(isOpen()).toBe(true)
    })

    it('opens and closes on click', async () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
      await flushPromises()

      toggle()
      await flushPromises()
      expect(isOpen()).toBe(true)

      toggle()
      await flushPromises()
      expect(isOpen()).toBe(false)
    })

    it('badges the unit Default until its content diverges from the generated default', async () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
      await flushPromises()
      const badge = (): string | undefined =>
        document.querySelector('.disclosure-head .badge')?.textContent?.trim()
      expect(badge()).toBe('Default')

      const textarea = document.querySelector<HTMLTextAreaElement>('textarea')!
      textarea.value = '# a hand-edited unit'
      textarea.dispatchEvent(new Event('input'))
      await flushPromises()

      expect(badge()).toBe('Customized')
    })
  })

  // Escape and the backdrop reach the dialog as BaseModal's close event, which
  // is wired separately from the Cancel button and has to behave the same.
  it.each([
    [
      'Cancel is clicked',
      (w: VueWrapper<ComponentPublicInstance>): void => {
        void w
        Array.from(document.querySelectorAll<HTMLButtonElement>('button'))
          .find((b) => b.textContent?.trim() === 'Cancel')
          ?.click()
      },
    ],
    [
      'the modal is dismissed',
      (w: VueWrapper<ComponentPublicInstance>): void => {
        w.findComponent(BaseModal).vm.$emit('close')
      },
    ],
  ])('emits close when %s', async (_how, dismiss) => {
    const w = mountDialog({ hostname: 'web-server-01', agentVersion: null })
    await flushPromises()
    dismiss(w)
    await w.vm.$nextTick()
    expect(w.emitted('close')).toHaveLength(1)
  })

  describe('deploy form', () => {
    /** The dialog teleports, so its fields are queried off the document. */
    async function setField(label: string, value: string): Promise<void> {
      const wrap = [...document.querySelectorAll('.field')].find((f) =>
        f.querySelector('.field-label')?.textContent?.includes(label),
      )
      const control = wrap?.querySelector<HTMLInputElement>('input, select, textarea')
      if (!control) throw new Error(`no field labelled "${label}"`)
      control.value = value
      control.dispatchEvent(new Event('input'))
      control.dispatchEvent(new Event('change'))
      await flushPromises()
    }

    async function submit(): Promise<void> {
      const button = [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) =>
        /Deploy|Install|Upgrade/.test(b.textContent ?? ''),
      )
      if (!button) throw new Error('no deploy button')
      button.click()
      await flushPromises()
    }

    it('sends every field it collected', async () => {
      const post = postMock
      post.mockResolvedValue({ data: { success: true, skipped: false } })

      mountDialog({ hostname: 'web-server-01', agentVersion: null })
      await flushPromises()
      post.mockClear()

      await setField('SSH host', '10.0.0.5')
      await setField('SSH user', 'deployer')
      await setField('SSH port', '2222')
      await setField('SSH password', 'hunter2')
      await setField('Server URL', 'https://assimilate.example.com')
      await setField('Install path', '/opt/assimilate')

      await submit()

      expect(post).toHaveBeenCalledWith(
        '/agents/web-server-01/deploy',
        expect.objectContaining({
          ssh_host: '10.0.0.5',
          ssh_user: 'deployer',
          ssh_port: 2222,
          ssh_password: 'hunter2',
          server_url: 'https://assimilate.example.com',
          install_path: '/opt/assimilate',
        }),
        { params: {} },
      )
    })

    // The optional fields are sent as undefined rather than as empty strings,
    // so the server applies its own defaults instead of writing blanks.
    it('omits the optional fields when they are left empty', async () => {
      const post = postMock
      post.mockResolvedValue({ data: { success: true, skipped: false } })

      mountDialog({ hostname: 'web-server-01', agentVersion: null })
      await flushPromises()
      post.mockClear()

      await setField('SSH host', '10.0.0.5')
      await setField('Server URL', 'https://assimilate.example.com')
      await setField('Install path', '')

      await submit()

      const body = post.mock.calls[0][1] as Record<string, unknown>
      expect(body.ssh_password).toBeUndefined()
      expect(body.install_path).toBeUndefined()
    })

    // After a successful deploy the form is replaced by the one-time token,
    // and the only way out is Done - Cancel and the deploy button are gone.
    it('shows the generated token and closes on Done', async () => {
      postMock.mockResolvedValue({
        data: { success: true, skipped: false, token: 'tok-abc123', available_version: '1.2.0' },
      })

      const w = mountDialog({ hostname: 'web-server-01', agentVersion: null })
      await flushPromises()

      await setField('SSH host', '10.0.0.5')
      await setField('Server URL', 'https://assimilate.example.com')
      await submit()

      expect(w.emitted('deployed')).toEqual([['1.2.0']])
      expect(document.querySelector('.token-text')?.textContent).toBe('tok-abc123')

      const done = [...document.querySelectorAll<HTMLButtonElement>('button')].find(
        (b) => b.textContent?.trim() === 'Done',
      )
      expect(done).toBeDefined()
      done!.click()
      await w.vm.$nextTick()

      expect(w.emitted('close')).toHaveLength(1)
    })

    it('reports a deploy failure rather than claiming success', async () => {
      const post = postMock
      post.mockResolvedValue({ data: {} })

      const w = mountDialog({ hostname: 'web-server-01', agentVersion: null })
      await flushPromises()
      post.mockRejectedValueOnce(new Error('ssh refused'))

      await setField('SSH host', '10.0.0.5')
      await setField('Server URL', 'https://assimilate.example.com')
      await submit()

      expect(w.emitted('deployed')).toBeUndefined()
      // extractError is stubbed to a fixed string in this file's mocks.
      expect(document.body.textContent).toContain('API error')
    })
  })
})
