// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
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

  describe('remote directory browser', () => {
    const DIRS = [
      { name: 'repos', is_dir: true },
      { name: 'notes.txt', is_dir: false },
      { name: 'archive', is_dir: true },
    ]

    function listDir(path: string, entries = DIRS) {
      return { data: { path, entries } }
    }

    async function openBrowser(path = '/backup'): Promise<void> {
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockResolvedValueOnce(listDir(path) as never)
      dialogButton('Browse').click()
      await flushPromises()
    }

    /**
     * Click the browser entry with the given label ('..' for the parent
     * entry, a directory name otherwise). Waits for the entry to actually
     * appear rather than querying once right after the triggering
     * `flushPromises()`: a single flush isn't always enough for the
     * reactive re-render to land before the next synchronous DOM query,
     * particularly under coverage instrumentation's extra scheduling
     * overhead, which previously made this an intermittent failure.
     */
    async function clickEntry(label: string): Promise<void> {
      const entry = await vi.waitFor(() => {
        const el = [...document.body.querySelectorAll<HTMLElement>('.entry-name')].find(
          (e) => e.textContent === label,
        )
        if (!el) throw new Error(`no browser entry labelled "${label}"`)
        return el
      })
      entry.parentElement?.click()
      await flushPromises()
    }

    it('will not browse before an SSH host is known', () => {
      mount()
      expect(dialogButton('Browse').disabled).toBe(true)
    })

    it('lists the remote directory over the entered SSH target', async () => {
      mount()
      await setField('SSH Host', 'backup.example.com')
      await setField('SSH User', 'operator')
      await setField('SSH Port', '2222')

      vi.mocked(apiClient.post).mockResolvedValueOnce(listDir('/') as never)
      dialogButton('Browse').click()
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/ssh/list-dir', {
        ssh_host: 'backup.example.com',
        ssh_user: 'operator',
        ssh_port: 2222,
        path: '/',
      })
    })

    // It is a directory picker: listing files would offer paths that cannot
    // hold a repository.
    it('shows only directories', async () => {
      mount()
      await openBrowser()
      const names = [...document.body.querySelectorAll('.entry-name')].map((e) => e.textContent)
      expect(names).toContain('repos')
      expect(names).toContain('archive')
      expect(names).not.toContain('notes.txt')
    })

    it('adopts the browsed directory as the repo path', async () => {
      mount()
      await openBrowser('/backup')
      expect((field('Repo Path') as HTMLInputElement).value).toBe('/backup')
    })

    it('descends into a directory on click', async () => {
      mount()
      await openBrowser('/backup')

      vi.mocked(apiClient.post).mockResolvedValueOnce(listDir('/backup/repos', []) as never)
      await clickEntry('repos')

      expect(apiClient.post).toHaveBeenLastCalledWith(
        '/ssh/list-dir',
        expect.objectContaining({ path: '/backup/repos' }),
      )
    })

    it('climbs to the parent on the .. entry', async () => {
      mount()
      await openBrowser('/backup/repos')

      vi.mocked(apiClient.post).mockResolvedValueOnce(listDir('/backup') as never)
      await clickEntry('..')

      expect(apiClient.post).toHaveBeenLastCalledWith(
        '/ssh/list-dir',
        expect.objectContaining({ path: '/backup' }),
      )
    })

    // The debounce used to outlive the dialog: closing it within 300ms of a
    // keystroke still fired the listing, and in tests that stray request
    // landed in whichever case happened to be running when the timer expired,
    // breaking its `toHaveBeenLastCalledWith`. It read as flakiness because
    // only a loaded machine pushed the timer past the end of its own test.
    it('cancels the pending path lookup when the dialog goes away', async () => {
      vi.useFakeTimers()
      try {
        const wrapper = mount()
        await setField('SSH Host', 'backup.example.com')
        await setField('Repo Path', '/backup/re')
        vi.mocked(apiClient.post).mockClear()

        wrapper.unmount()
        vi.advanceTimersByTime(1000)
        await flushPromises()

        expect(apiClient.post).not.toHaveBeenCalled()
      } finally {
        vi.useRealTimers()
      }
    })

    it('looks the path up once the debounce elapses', async () => {
      vi.useFakeTimers()
      try {
        mount()
        await setField('SSH Host', 'backup.example.com')
        await setField('Repo Path', '/backup/re')
        vi.mocked(apiClient.post)
          .mockClear()
          .mockResolvedValue(listDir('/backup') as never)

        vi.advanceTimersByTime(300)
        await flushPromises()

        expect(apiClient.post).toHaveBeenCalledWith(
          '/ssh/list-dir',
          expect.objectContaining({ path: '/backup' }),
        )
      } finally {
        vi.useRealTimers()
      }
    })

    it('jumps to an ancestor from the breadcrumbs', async () => {
      mount()
      await openBrowser('/backup/repos')

      vi.mocked(apiClient.post).mockResolvedValueOnce(listDir('/backup') as never)
      const crumbs = [...document.body.querySelectorAll<HTMLElement>('.crumb')]
      crumbs[1]?.click()
      await flushPromises()

      expect(apiClient.post).toHaveBeenLastCalledWith(
        '/ssh/list-dir',
        expect.objectContaining({ path: expect.stringContaining('/backup') }),
      )
    })

    it('reports a listing error from the server rather than showing an empty directory', async () => {
      mount()
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { path: '/root', entries: [], error: 'Permission denied' },
      } as never)
      dialogButton('Browse').click()
      await flushPromises()

      expect(document.body.textContent).toContain('Permission denied')
    })

    it('reports a failed listing request', async () => {
      mount()
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('ssh timeout'))
      dialogButton('Browse').click()
      await flushPromises()

      expect(document.body.querySelector('.browser-panel')?.textContent).toBeTruthy()
    })
  })

  describe('new folder', () => {
    async function openFolderDialog(): Promise<void> {
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { path: '/backup', entries: [] },
      } as never)
      dialogButton('Browse').click()
      await flushPromises()
      dialogButton('New Folder').click()
      await flushPromises()
    }

    it('refuses an empty name instead of creating one remotely', async () => {
      mount({ mode: 'create' })
      await openFolderDialog()
      vi.mocked(apiClient.post).mockClear()

      dialogButton('Create').click()
      await flushPromises()

      expect(apiClient.post).not.toHaveBeenCalled()
      expect(document.body.textContent).toContain('Folder name is required')
    })

    /**
     * The folder prompt is a second BaseModal stacked on the create dialog,
     * so the last `.modal-dialog` is the one that owns the name field - the
     * first is still the repository form behind it.
     */
    async function typeFolderName(name: string): Promise<void> {
      const dialogs = [...document.body.querySelectorAll('.modal-dialog')]
      const input = dialogs[dialogs.length - 1]!.querySelector<HTMLInputElement>('input')!
      input.value = name
      input.dispatchEvent(new Event('input'))
      await flushPromises()
    }

    it('creates the folder under the browsed directory and opens it', async () => {
      mount({ mode: 'create' })
      await openFolderDialog()
      await typeFolderName('nightly')
      vi.mocked(apiClient.post).mockClear()

      vi.mocked(apiClient.post)
        .mockResolvedValueOnce({} as never)
        .mockResolvedValueOnce({ data: { path: '/backup/nightly', entries: [] } } as never)
      dialogButton('Create').click()
      await flushPromises()

      expect(apiClient.post).toHaveBeenNthCalledWith(1, '/ssh/mkdir', {
        ssh_host: 'backup.example.com',
        ssh_user: 'borg',
        ssh_port: 22,
        path: '/backup/nightly',
      })
      // Creating it is only half the job - the browser should land in it.
      expect(apiClient.post).toHaveBeenNthCalledWith(
        2,
        '/ssh/list-dir',
        expect.objectContaining({ path: '/backup/nightly' }),
      )
    })

    it('keeps the dialog open with the error when the folder cannot be created', async () => {
      mount({ mode: 'create' })
      await openFolderDialog()
      await typeFolderName('nightly')

      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('Read-only file system'))
      dialogButton('Create').click()
      await flushPromises()

      expect(document.body.textContent).toContain('Read-only file system')
    })
  })

  describe('path autocomplete', () => {
    const ENTRIES = [
      { name: 'repos', is_dir: true },
      { name: 'reports', is_dir: true },
      { name: 'archive', is_dir: true },
      { name: 'readme.txt', is_dir: false },
    ]

    /** Types into the repo path field and lets the 300ms debounce elapse. */
    async function typePath(value: string): Promise<void> {
      const input = field('Repo Path') as HTMLInputElement
      input.value = value
      input.dispatchEvent(new Event('input'))
      await vi.advanceTimersByTimeAsync(350)
      await flushPromises()
    }

    beforeEach(() => {
      vi.useFakeTimers()
    })

    afterEach(() => {
      vi.useRealTimers()
    })

    it('suggests nothing until an SSH host is known', async () => {
      mount()
      await typePath('/backup/re')
      expect(apiClient.post).not.toHaveBeenCalled()
      expect(document.body.querySelector('.autocomplete-dropdown')).toBeNull()
    })

    it('cancels the pending debounce when the dialog closes', async () => {
      // The debounce used to outlive the component, so closing the dialog within
      // 300ms of typing still fired an /ssh/list-dir request (and could trigger a
      // browseDir) for a dialog that no longer existed. In this suite that stray
      // request also consumed the next queued mockResolvedValueOnce, which left a
      // later test's browse unmocked, put the browser into its error state, and
      // made the directory listing - and the ".." entry a test then clicks -
      // disappear. That is what made the browser tests intermittently fail.
      const wrapper = mount()
      await setField('SSH Host', 'backup.example.com')

      const input = field('Repo Path') as HTMLInputElement
      input.value = '/backup/repos/web'
      input.dispatchEvent(new Event('input'))

      wrapper.unmount()
      vi.mocked(apiClient.post).mockClear()

      await vi.advanceTimersByTimeAsync(1000)
      await flushPromises()

      expect(apiClient.post).not.toHaveBeenCalled()
    })

    it('lists the parent directory and filters by the typed prefix', async () => {
      mount()
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { path: '/backup', entries: ENTRIES },
      } as never)

      await typePath('/backup/re')

      expect(apiClient.post).toHaveBeenCalledWith(
        '/ssh/list-dir',
        expect.objectContaining({ path: '/backup' }),
      )
      const shown = [...document.body.querySelectorAll('.autocomplete-item')].map((i) =>
        i.textContent?.trim(),
      )
      // Prefix-matched and directories only: "archive" fails the prefix and
      // "readme.txt" is not a directory.
      expect(shown).toEqual(['repos', 'reports'])
    })

    it('completes the last path segment rather than appending to it', async () => {
      mount()
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { path: '/backup', entries: ENTRIES },
      } as never)
      await typePath('/backup/re')

      const item = [...document.body.querySelectorAll<HTMLElement>('.autocomplete-item')].find(
        (i) => i.textContent?.trim() === 'reports',
      )
      item?.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
      await flushPromises()

      expect((field('Repo Path') as HTMLInputElement).value).toBe('/backup/reports')
      expect(document.body.querySelector('.autocomplete-dropdown')).toBeNull()
    })

    it('offers nothing when the directory has no matching child', async () => {
      mount()
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { path: '/backup', entries: ENTRIES },
      } as never)

      await typePath('/backup/zzz')

      expect(document.body.querySelector('.autocomplete-dropdown')).toBeNull()
    })

    it('stays quiet when the listing reports an error', async () => {
      mount()
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { path: '/backup', entries: [], error: 'Permission denied' },
      } as never)

      await typePath('/backup/re')

      expect(document.body.querySelector('.autocomplete-dropdown')).toBeNull()
    })

    it('stays quiet when the listing request fails outright', async () => {
      mount()
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockRejectedValue(new Error('ssh timeout'))

      await typePath('/backup/re')

      expect(document.body.querySelector('.autocomplete-dropdown')).toBeNull()
    })

    it('clears the suggestions once the field is emptied', async () => {
      mount()
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { path: '/backup', entries: ENTRIES },
      } as never)
      await typePath('/backup/re')
      expect(document.body.querySelector('.autocomplete-dropdown')).not.toBeNull()

      await typePath('')

      expect(document.body.querySelector('.autocomplete-dropdown')).toBeNull()
    })

    it('dismisses the suggestions shortly after the field loses focus', async () => {
      mount()
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { path: '/backup', entries: ENTRIES },
      } as never)
      await typePath('/backup/re')
      ;(field('Repo Path') as HTMLInputElement).dispatchEvent(new Event('blur'))
      await vi.advanceTimersByTimeAsync(250)
      await flushPromises()

      expect(document.body.querySelector('.autocomplete-dropdown')).toBeNull()
    })

    // Typing a trailing slash means "show me inside this directory", so an
    // open browser panel should follow the field rather than go stale.
    it('walks an open browser panel to a directory typed with a trailing slash', async () => {
      mount()
      await setField('SSH Host', 'backup.example.com')
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { path: '/backup', entries: ENTRIES },
      } as never)
      dialogButton('Browse').click()
      await flushPromises()

      vi.mocked(apiClient.post).mockClear()
      await typePath('/backup/repos/')

      expect(apiClient.post).toHaveBeenCalledWith(
        '/ssh/list-dir',
        expect.objectContaining({ path: '/backup/repos' }),
      )
    })
  })

  it('reports a connection test that throws', async () => {
    mount()
    await setField('SSH Host', 'backup.example.com')
    vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('no route to host'))

    dialogButton('Test Connection').click()
    await flushPromises()

    expect(document.body.textContent).toContain('no route to host')
  })

  it('sends the chosen encryption and compression', async () => {
    const wrapper = mount({ mode: 'create' })
    await fillValidForm()
    await setField('Encryption', 'keyfile')
    await setField('Compression', 'zstd')

    dialogButton('Create Repo').click()
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({ encryption: 'keyfile', compression: 'zstd' }),
    )
    expect(wrapper.exists()).toBe(true)
  })

  it('toggles the deploy key panel', async () => {
    mount()
    await setField('SSH Host', 'backup.example.com')
    const before = document.body.textContent ?? ''
    dialogButton('+ Deploy Key').click()
    await flushPromises()
    expect(document.body.textContent).not.toBe(before)
  })

  it('closes the new-folder prompt on its Cancel without creating anything', async () => {
    mount({ mode: 'create' })
    await setField('SSH Host', 'backup.example.com')
    vi.mocked(apiClient.post).mockResolvedValueOnce({
      data: { path: '/backup', entries: [] },
    } as never)
    dialogButton('Browse').click()
    await flushPromises()
    dialogButton('New Folder').click()
    await flushPromises()
    vi.mocked(apiClient.post).mockClear()

    const dialogs = [...document.body.querySelectorAll('.modal-dialog')]
    const cancel = [
      ...dialogs[dialogs.length - 1]!.querySelectorAll<HTMLButtonElement>('button'),
    ].find((b) => b.textContent?.trim() === 'Cancel')
    cancel?.click()
    await flushPromises()

    expect(apiClient.post).not.toHaveBeenCalled()
  })

  it('closes on the footer Cancel', async () => {
    const wrapper = mount()
    dialogButton('Cancel').click()
    await flushPromises()
    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('closes on the modal dismiss control as well as the footer', async () => {
    const wrapper = mount()
    document.body.querySelector<HTMLButtonElement>('.modal-close')?.click()
    await flushPromises()
    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('dismisses the new-folder prompt on its own modal control, leaving the form open', async () => {
    const wrapper = mount({ mode: 'create' })
    await setField('SSH Host', 'backup.example.com')
    vi.mocked(apiClient.post).mockResolvedValueOnce({
      data: { path: '/backup', entries: [] },
    } as never)
    dialogButton('Browse').click()
    await flushPromises()
    dialogButton('New Folder').click()
    await flushPromises()
    expect(document.body.querySelectorAll('.modal-dialog')).toHaveLength(2)

    const closes = [...document.body.querySelectorAll<HTMLButtonElement>('.modal-close')]
    closes[closes.length - 1]?.click()
    await flushPromises()

    // Only the inner prompt goes away; dismissing it must not take the
    // half-filled repository form with it.
    expect(document.body.querySelectorAll('.modal-dialog')).toHaveLength(1)
    expect(wrapper.emitted('close')).toBeUndefined()
  })
})
