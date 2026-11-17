// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import RepoCreateDialog from './RepoCreateDialog.vue'
import type { ProviderPreset, RepoHostResponse } from '../types/generated'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn() },
}))

const PRESETS: ProviderPreset[] = [
  {
    provider: 'generic',
    defaults: {
      ssh_port: null,
      hostname_suffix: null,
      base_path: null,
      remote_borg: null,
      key_deploy: 'sftp',
    },
  },
  {
    provider: 'hetzner',
    defaults: {
      ssh_port: 23,
      hostname_suffix: '.your-storagebox.de',
      base_path: './backups',
      remote_borg: 'borg-1.4',
      key_deploy: 'sftp',
    },
  },
  {
    provider: 'rsync_net',
    defaults: {
      ssh_port: 22,
      hostname_suffix: null,
      base_path: null,
      remote_borg: 'borg14',
      key_deploy: 'sftp',
    },
  },
  {
    provider: 'borgbase',
    defaults: {
      ssh_port: 22,
      hostname_suffix: null,
      base_path: null,
      remote_borg: null,
      key_deploy: 'shell',
    },
  },
]

const HOSTS = [
  { id: 4, ssh_host: 'u9.your-storagebox.de', ssh_port: 23, provider: 'hetzner' },
] as unknown as RepoHostResponse[]

async function open(): Promise<void> {
  const wrapper = renderWithPlugins(RepoCreateDialog, {
    props: { open: true, mode: 'import', repos: [] },
  })
  // The view calls reset() each time it opens the dialog, which loads the presets.
  ;(wrapper.vm as unknown as { reset: () => void }).reset()
  await flushPromises()
}

function control(id: string): HTMLInputElement | HTMLSelectElement {
  const el = document.body.querySelector<HTMLInputElement | HTMLSelectElement>(id)
  if (!el) throw new Error(`no control ${id}`)
  return el
}

function byLabel(label: string): HTMLInputElement {
  const field = [...document.body.querySelectorAll('.field, .browser-header')].find((f) =>
    f.querySelector('.field-label')?.textContent?.includes(label),
  )
  const el = field?.querySelector('input')
  if (!el) throw new Error(`no field ${label}`)
  return el
}

async function pick(provider: string): Promise<void> {
  const select = control('#repo-provider') as HTMLSelectElement
  select.value = provider
  select.dispatchEvent(new Event('change'))
  await flushPromises()
}

async function type(el: HTMLInputElement, value: string): Promise<void> {
  el.value = value
  el.dispatchEvent(new Event('input'))
  await flushPromises()
}

describe('RepoCreateDialog provider presets', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    vi.mocked(apiClient.post)
      .mockReset()
      .mockResolvedValue({ data: { id: 9 } } as never)
    vi.mocked(apiClient.get)
      .mockReset()
      .mockImplementation(async (url: string) => ({
        data: url === '/repo-hosts/providers' ? PRESETS : HOSTS,
      }))
  })

  it('offers every provider, generic first', async () => {
    await open()
    const options = [...(control('#repo-provider') as HTMLSelectElement).options].map((o) =>
      o.textContent?.trim(),
    )
    expect(options).toEqual(['Generic SSH', 'Hetzner Storage Box', 'rsync.net', 'BorgBase'])
  })

  it('is the plain generic form when the presets cannot be loaded', async () => {
    vi.mocked(apiClient.get).mockReset().mockRejectedValue(new Error('offline'))
    await open()
    expect(document.body.querySelector('#repo-provider')).toBeNull()
    expect(byLabel('SSH port').disabled).toBe(false)
  })

  it('locks the Hetzner port, completes the hostname and fills the backups directory', async () => {
    await open()
    await pick('hetzner')

    const port = byLabel('SSH port')
    expect(port.value).toBe('23')
    expect(port.disabled).toBe(true)

    const host = byLabel('SSH host')
    await type(host, 'u123456')
    host.dispatchEvent(new Event('blur'))
    await flushPromises()
    expect(host.value).toBe('u123456.your-storagebox.de')

    expect(document.body.textContent).toContain('borg-1.4')
  })

  it('fills the backups directory only while the path is empty', async () => {
    await open()
    await pick('hetzner')
    expect(byLabel('Repo path').value).toBe('./backups/')

    await pick('generic')
    await type(byLabel('Repo path'), '/srv/own')
    await pick('hetzner')
    expect(byLabel('Repo path').value).toBe('/srv/own')
  })

  it('gives the typed port back when the provider goes back to generic', async () => {
    await open()
    await type(byLabel('SSH port'), '2222')
    await pick('hetzner')
    expect(byLabel('SSH port').value).toBe('23')

    await pick('generic')
    expect(byLabel('SSH port').value).toBe('2222')
    expect(byLabel('SSH port').disabled).toBe(false)
  })

  it('names rsync.net remote borg and leaves the hostname alone', async () => {
    await open()
    await pick('rsync_net')
    expect(document.body.textContent).toContain('borg14')

    const host = byLabel('SSH host')
    await type(host, 'ab123.rsync.net')
    host.dispatchEvent(new Event('blur'))
    await flushPromises()
    expect(host.value).toBe('ab123.rsync.net')
  })

  it('locks the provider to the one an existing host already has', async () => {
    await open()
    await type(byLabel('SSH host'), 'u9.your-storagebox.de')

    const select = control('#repo-provider') as HTMLSelectElement
    expect(select.value).toBe('hetzner')
    expect(select.disabled).toBe(true)
  })

  it('gives the provider back when the hostname moves off a known host', async () => {
    await open()
    await pick('rsync_net')
    const host = byLabel('SSH host')

    await type(host, 'u9.your-storagebox.de')
    const select = control('#repo-provider') as HTMLSelectElement
    expect(select.value).toBe('hetzner')
    expect(byLabel('SSH port').value).toBe('23')

    await type(host, 'brand-new.example.org')
    expect(select.disabled).toBe(false)
    expect(select.value).toBe('rsync_net')
    expect(byLabel('SSH port').value).toBe('22')
  })

  it('starts a reopened dialog clean even after a known host locked it', async () => {
    const wrapper = renderWithPlugins(RepoCreateDialog, {
      props: { open: true, mode: 'import', repos: [] },
    })
    const reset = (wrapper.vm as unknown as { reset: () => void }).reset
    reset()
    await flushPromises()

    // A provider picked first, then a hostname that belongs to another one.
    await pick('rsync_net')
    await type(byLabel('SSH host'), 'u9.your-storagebox.de')
    expect((control('#repo-provider') as HTMLSelectElement).value).toBe('hetzner')

    // Closed with the host still typed, then opened again.
    reset()
    await flushPromises()
    expect((control('#repo-provider') as HTMLSelectElement).value).toBe('generic')
    expect(byLabel('SSH port').value).toBe('22')
    expect(byLabel('SSH port').disabled).toBe(false)
  })

  it('only promises SFTP when the provider deploys over SFTP', async () => {
    await open()
    await pick('hetzner')
    expect(document.body.textContent).toContain('deployed over SFTP')

    await pick('borgbase')
    expect(document.body.textContent).not.toContain('deployed over SFTP')
  })

  it('sends the chosen provider with the repository', async () => {
    await open()
    await pick('rsync_net')
    await type(byLabel('Name'), 'offsite')
    await type(byLabel('SSH host'), 'ab123.rsync.net')
    await type(byLabel('Repo path'), 'backups/web')
    await type(byLabel('Passphrase'), 'secret')

    const submit = [...document.body.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === 'Import Repo',
    )
    submit?.click()
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith(
      '/repos',
      expect.objectContaining({ provider: 'rsync_net', ssh_port: 22 }),
    )
  })

  it('sends no provider for a generic host', async () => {
    await open()
    await type(byLabel('Name'), 'plain')
    await type(byLabel('SSH host'), 'nas.lan')
    await type(byLabel('Repo path'), '/srv/b')
    await type(byLabel('Passphrase'), 'secret')

    const submit = [...document.body.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === 'Import Repo',
    )
    submit?.click()
    await flushPromises()

    const body = vi.mocked(apiClient.post).mock.calls[0]?.[1] as Record<string, unknown>
    expect('provider' in body).toBe(false)
  })
})
