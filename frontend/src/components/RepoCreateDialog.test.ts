// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import RepoCreateDialog from './RepoCreateDialog.vue'
import type { RepoWithStats } from '../types/repo'

vi.mock('../api/client', () => ({
  apiClient: { post: vi.fn() },
}))

const EXISTING = [
  { id: 1, ssh_user: 'borg', ssh_host: 'backup.example.com', ssh_port: 22 },
  { id: 2, ssh_user: 'borg', ssh_host: 'backup.example.com', ssh_port: 22 },
  { id: 3, ssh_user: 'root', ssh_host: 'other.example.com', ssh_port: 2222 },
] as unknown as RepoWithStats[]

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoCreateDialog, {
    props: { open: true, mode: 'import', repos: EXISTING, ...props },
  })
}

/** The dialog teleports, so its fields are queried off the document body. */
function field(label: string): HTMLInputElement | HTMLSelectElement {
  const labels = [...document.body.querySelectorAll('.field, .browser-header')]
  const match = labels.find((f) => f.querySelector('.field-label')?.textContent?.includes(label))
  const control = match?.querySelector('input, select')
  if (!control) throw new Error(`no field labelled "${label}"`)
  return control as HTMLInputElement
}

async function setField(label: string, value: string): Promise<void> {
  const control = field(label)
  control.value = value
  control.dispatchEvent(new Event('input'))
  control.dispatchEvent(new Event('change'))
  await flushPromises()
}

function dialogButton(label: string): HTMLButtonElement {
  const match = [...document.body.querySelectorAll<HTMLButtonElement>('button')].find(
    (b) => b.textContent?.trim() === label,
  )
  if (!match) throw new Error(`no button labelled "${label}"`)
  return match
}

async function fillValidForm(): Promise<void> {
  await setField('Name', 'inhouse')
  await setField('SSH Host', 'backup.example.com')
  await setField('Repo Path', '/backup/repos/web')
  await setField('Passphrase', 'correct horse')
}

describe('RepoCreateDialog', () => {
  beforeEach(() => {
    vi.mocked(apiClient.post)
      .mockReset()
      .mockResolvedValue({ data: { id: 9 } } as never)
  })

  it('titles itself by the flow it is in', () => {
    mount({ mode: 'import' })
    expect(document.body.textContent).toContain('Import Repository')

    document.body.innerHTML = ''
    mount({ mode: 'create' })
    expect(document.body.textContent).toContain('Create Repository')
  })

  it('offers each distinct SSH target once', () => {
    mount()
    const options = [...(field('Fill SSH from existing') as HTMLSelectElement).options].map(
      (o) => o.value,
    )
    expect(options).toEqual(['', 'borg@backup.example.com:22', 'root@other.example.com:2222'])
  })

  it('fills the SSH fields from a chosen target', async () => {
    mount()
    await setField('Fill SSH from existing', 'root@other.example.com:2222')
    expect((field('SSH User') as HTMLInputElement).value).toBe('root')
    expect((field('SSH Host') as HTMLInputElement).value).toBe('other.example.com')
    expect((field('SSH Port') as HTMLInputElement).value).toBe('2222')
  })

  it('keeps the submit button disabled until every required field is filled', async () => {
    mount()
    expect(dialogButton('Import Repo').disabled).toBe(true)
    await fillValidForm()
    expect(dialogButton('Import Repo').disabled).toBe(false)
  })

  it('adopts an existing repository and reports the new row', async () => {
    const wrapper = mount({ mode: 'import' })
    await fillValidForm()
    dialogButton('Import Repo').click()
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/repos', {
      name: 'inhouse',
      repo_path: '/backup/repos/web',
      ssh_user: 'borg',
      ssh_host: 'backup.example.com',
      ssh_port: 22,
      passphrase: 'correct horse',
      compression: 'lz4',
    })
    expect(wrapper.emitted('imported')).toEqual([[{ id: 9 }]])
    expect(wrapper.emitted('close')).toHaveLength(1)
    expect(wrapper.emitted('created')).toBeUndefined()
  })

  it('initialises a new repository and asks the caller to refetch', async () => {
    const wrapper = mount({ mode: 'create' })
    await fillValidForm()
    dialogButton('Create Repo').click()
    await flushPromises()

    expect(vi.mocked(apiClient.post).mock.calls[0][0]).toBe('/repos/init')
    expect(vi.mocked(apiClient.post).mock.calls[0][1]).toMatchObject({
      encryption: 'repokey-blake2',
    })
    expect(wrapper.emitted('created')).toHaveLength(1)
    expect(wrapper.emitted('imported')).toBeUndefined()
  })

  it('offers the encryption mode only when creating, since import inherits it', () => {
    mount({ mode: 'create' })
    expect(document.body.textContent).toContain('Encryption')

    document.body.innerHTML = ''
    mount({ mode: 'import' })
    expect(document.body.textContent).not.toContain('Encryption')
  })

  it('shows the server error and stays open when the request fails', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('repo path already registered'))
    const wrapper = mount()
    await fillValidForm()
    dialogButton('Import Repo').click()
    await flushPromises()

    expect(document.body.textContent).toContain('repo path already registered')
    expect(wrapper.emitted('close')).toBeUndefined()
  })

  it('runs the SSH connection test against the entered target', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { ssh_ok: true, borg_installed: true, borg_version: '1.4.0' },
    } as never)
    mount()
    await setField('SSH Host', 'backup.example.com')
    dialogButton('Test Connection').click()
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/ssh/test-connection', {
      ssh_host: 'backup.example.com',
      ssh_user: 'borg',
      ssh_port: 22,
    })
    expect(document.body.textContent).toContain('SSH OK, borg 1.4.0')
  })

  it('distinguishes a reachable host with no borg from an unreachable one', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { ssh_ok: true, borg_installed: false },
    } as never)
    mount()
    await setField('SSH Host', 'backup.example.com')
    dialogButton('Test Connection').click()
    await flushPromises()
    expect(document.body.textContent).toContain('SSH OK, borg not found')
  })

  it('will not test or browse before a host is entered', () => {
    mount()
    expect(dialogButton('Test Connection').disabled).toBe(true)
    expect(dialogButton('Browse').disabled).toBe(true)
  })

  it('lists only directories when browsing, and adopts the path browsed to', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: {
        path: '/backup',
        entries: [
          { name: 'repos', is_dir: true },
          { name: 'notes.txt', is_dir: false },
        ],
      },
    } as never)
    mount()
    await setField('SSH Host', 'backup.example.com')
    dialogButton('Browse').click()
    await flushPromises()

    const entries = [...document.body.querySelectorAll('.entry-name')].map((e) => e.textContent)
    // '..' is the parent-directory row; the plain file is filtered out.
    expect(entries).toEqual(['..', 'repos'])
    expect((field('Repo Path') as HTMLInputElement).value).toBe('/backup')
  })

  it('surfaces a browse failure in the panel rather than the form error', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { path: '/', entries: [], error: 'Permission denied' },
    } as never)
    mount()
    await setField('SSH Host', 'backup.example.com')
    dialogButton('Browse').click()
    await flushPromises()

    expect(document.body.querySelector('.browser-error')?.textContent).toContain(
      'Permission denied',
    )
  })

  it('offers New Folder only in the create flow', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { path: '/', entries: [] } } as never)
    mount({ mode: 'create' })
    await setField('SSH Host', 'backup.example.com')
    dialogButton('Browse').click()
    await flushPromises()
    expect(document.body.textContent).toContain('New Folder')

    document.body.innerHTML = ''
    mount({ mode: 'import' })
    await setField('SSH Host', 'backup.example.com')
    dialogButton('Browse').click()
    await flushPromises()
    expect(document.body.textContent).not.toContain('New Folder')
  })

  it('clears the form when the view reopens it', async () => {
    const wrapper = mount()
    await setField('Name', 'stale')
    ;(wrapper.vm as unknown as { reset: () => void }).reset()
    await flushPromises()
    expect((field('Name') as HTMLInputElement).value).toBe('')
  })
})
