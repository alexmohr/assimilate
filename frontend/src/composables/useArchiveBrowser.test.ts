// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    delete: vi.fn(),
  },
}))

import { apiClient } from '../api/client'
import { useArchiveBrowser, type ArchiveEntry, type ContentEntry } from './useArchiveBrowser'

const ARCHIVE: ArchiveEntry = {
  name: 'web-server-01-backup-2026-06-05T02:00:00',
  start: '2026-06-05T02:00:00',
  hostname: 'imported-hostname',
  comment: '',
  original_size: 1,
  deduplicated_size: 1,
  matched: true,
  agent_hostname: 'web-server-01',
}

const ROOT_ENTRY: ContentEntry = {
  type: 'd',
  path: '',
  size: 0,
  mtime: '',
  mode: '',
}

describe('useArchiveBrowser', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.stubGlobal(
      'confirm',
      vi.fn(() => true),
    )
  })

  it('downloads the root entry as the whole archive without an empty path query', () => {
    const browser = useArchiveBrowser(ref(5))
    browser.selectedArchive.value = ARCHIVE
    const anchor = document.createElement('a')
    vi.spyOn(document, 'createElement').mockReturnValue(anchor)
    vi.spyOn(anchor, 'click').mockImplementation(() => undefined)

    browser.downloadEntry(ROOT_ENTRY)

    expect(anchor.getAttribute('href')).toBe(
      '/api/repos/5/archives/web-server-01-backup-2026-06-05T02%3A00%3A00/export',
    )
  })

  it('restores the root entry as the whole archive to its matched host', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { success: true } })
    const browser = useArchiveBrowser(ref(5))
    browser.selectedArchive.value = ARCHIVE

    await browser.restoreEntry(ROOT_ENTRY)

    expect(apiClient.post).toHaveBeenCalledWith(
      '/repos/5/archives/web-server-01-backup-2026-06-05T02%3A00%3A00/restore',
      {
        paths: [],
        target_path: '/',
        hostname: 'web-server-01',
      },
    )
  })

  it('restores one entry by passing its archive path', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { success: true } })
    const browser = useArchiveBrowser(ref(5))
    browser.selectedArchive.value = ARCHIVE
    const entry = { ...ROOT_ENTRY, type: '-', path: 'etc/nginx/nginx.conf' }

    await browser.restoreEntry(entry)

    expect(apiClient.post).toHaveBeenCalledWith(expect.any(String), {
      paths: ['etc/nginx/nginx.conf'],
      target_path: '/',
      hostname: 'web-server-01',
    })
  })

  it('deletes the whole archive from the root entry', async () => {
    const confirm = vi.mocked(window.confirm)
    vi.mocked(apiClient.delete).mockResolvedValue({
      data: { success: true, archive_name: ARCHIVE.name },
    })

    const browser = useArchiveBrowser(ref(5))
    browser.selectedArchive.value = ARCHIVE
    browser.archives.value = [ARCHIVE]

    await browser.deleteArchive(ROOT_ENTRY)

    expect(apiClient.delete).toHaveBeenCalledWith(
      '/repos/5/archives/web-server-01-backup-2026-06-05T02%3A00%3A00',
    )
    // Deletion runs in the background; the archive stays in the list until a
    // DataChanged event confirms borg finished. Only the open detail pane closes.
    expect(browser.selectedArchive.value).toBeNull()
    expect(browser.archives.value).toHaveLength(1)
    expect(confirm).not.toHaveBeenCalled()
  })

  it('deleteArchiveByName keeps the list until DataChanged and does not require selectedArchive', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({
      data: { success: true, archive_name: ARCHIVE.name },
    })

    const SECOND: ArchiveEntry = { ...ARCHIVE, name: 'web-server-01-backup-2026-06-06T02:00:00' }
    const browser = useArchiveBrowser(ref(5))
    browser.archives.value = [ARCHIVE, SECOND]

    await browser.deleteArchiveByName(ARCHIVE)

    expect(apiClient.delete).toHaveBeenCalledWith(
      '/repos/5/archives/web-server-01-backup-2026-06-05T02%3A00%3A00',
    )
    expect(browser.archives.value).toEqual([ARCHIVE, SECOND])
  })

  it('deleteArchiveByName clears selectedArchive when it matches the deleted archive', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({
      data: { success: true, archive_name: ARCHIVE.name },
    })

    const browser = useArchiveBrowser(ref(5))
    browser.archives.value = [ARCHIVE]
    browser.selectedArchive.value = ARCHIVE

    await browser.deleteArchiveByName(ARCHIVE)

    expect(browser.selectedArchive.value).toBeNull()
    expect(browser.archives.value).toHaveLength(1)
  })

  it('browserEntries maps root directory and file entries with displayName and isDir', () => {
    const browser = useArchiveBrowser(ref(5))
    browser.currentPath.value = '/'
    browser.contents.value = [
      { type: 'd', path: '', size: 0, mtime: '2026-01-01T00:00:00Z', mode: '755' },
      { type: '-', path: 'file.txt', size: 1024, mtime: '2026-01-01T00:00:00Z', mode: '644' },
      { type: 'd', path: 'subdir', size: 0, mtime: '2026-01-01T00:00:00Z', mode: '755' },
    ]

    const entries = browser.browserEntries.value
    expect(entries).toHaveLength(3)
    expect(entries[0].displayName).toBe('.')
    expect(entries[0].isDir).toBe(true)
    expect(entries[1].displayName).toBe('subdir')
    expect(entries[1].isDir).toBe(true)
    expect(entries[2].displayName).toBe('file.txt')
    expect(entries[2].isDir).toBe(false)
    expect(entries[2].size).toBe(1024)
    expect(entries[2].mtime).toBe('2026-01-01T00:00:00Z')
  })

  it('browserEntries includes ".." entry when not at root', () => {
    const browser = useArchiveBrowser(ref(5))
    browser.currentPath.value = '/subdir'
    browser.contents.value = [
      { type: 'd', path: 'subdir', size: 0, mtime: '', mode: '755' },
      { type: '-', path: 'subdir/nested.txt', size: 512, mtime: '', mode: '644' },
    ]

    const entries = browser.browserEntries.value
    expect(entries).toHaveLength(3)
    const dotDir = entries.find((e) => e.displayName === '.')
    expect(dotDir).toBeTruthy()
    expect(dotDir!.isDir).toBe(true)
    const dotdot = entries.find((e) => e.displayName === '..')
    expect(dotdot).toBeTruthy()
    expect(dotdot!.isDir).toBe(true)
    const nestedFile = entries.find((e) => e.displayName === 'nested.txt')
    expect(nestedFile).toBeTruthy()
    expect(nestedFile!.isDir).toBe(false)
  })

  it('browserEntries shows only "." when at root with no content entries', () => {
    const browser = useArchiveBrowser(ref(5))
    browser.contents.value = []
    browser.currentPath.value = '/'

    const entries = browser.browserEntries.value
    expect(entries).toHaveLength(1)
    expect(entries[0].displayName).toBe('.')
    expect(entries[0].isDir).toBe(true)
  })

  it('loadArchives fetches archives and clears errors', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [ARCHIVE] })

    const browser = useArchiveBrowser(ref(5))
    await browser.loadArchives()

    expect(browser.archives.value).toEqual([ARCHIVE])
    expect(browser.archivesError.value).toBeNull()
    expect(browser.archivesLoading.value).toBe(false)
  })

  it('loadArchives sets an error when the API call fails', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('network error'))

    const browser = useArchiveBrowser(ref(5))
    await browser.loadArchives()

    expect(browser.archivesError.value).toBe('network error')
    expect(browser.archivesLoading.value).toBe(false)
  })

  it('loadArchives sets archivesLoading while the request is in flight', async () => {
    let resolveGet: (value: { data: ArchiveEntry[] }) => void = () => {}
    vi.mocked(apiClient.get).mockReturnValue(
      new Promise((resolve) => {
        resolveGet = resolve
      }),
    )

    const browser = useArchiveBrowser(ref(5))
    const pending = browser.loadArchives()

    expect(browser.archivesLoading.value).toBe(true)
    resolveGet({ data: [ARCHIVE] })
    await pending

    expect(browser.archivesLoading.value).toBe(false)
  })

  it('loadArchives(true) fetches silently, without ever setting archivesLoading', async () => {
    let resolveGet: (value: { data: ArchiveEntry[] }) => void = () => {}
    vi.mocked(apiClient.get).mockReturnValue(
      new Promise((resolve) => {
        resolveGet = resolve
      }),
    )

    const browser = useArchiveBrowser(ref(5))
    const pending = browser.loadArchives(true)

    expect(browser.archivesLoading.value).toBe(false)
    resolveGet({ data: [ARCHIVE] })
    await pending

    expect(browser.archives.value).toEqual([ARCHIVE])
    expect(browser.archivesLoading.value).toBe(false)
  })

  it('loadArchives discards a stale response that resolves after a newer call', async () => {
    let resolveFirst: (value: { data: ArchiveEntry[] }) => void = () => {}
    let resolveSecond: (value: { data: ArchiveEntry[] }) => void = () => {}
    vi.mocked(apiClient.get)
      .mockReturnValueOnce(
        new Promise((resolve) => {
          resolveFirst = resolve
        }),
      )
      .mockReturnValueOnce(
        new Promise((resolve) => {
          resolveSecond = resolve
        }),
      )

    const browser = useArchiveBrowser(ref(5))
    const firstCall = browser.loadArchives()
    const secondCall = browser.loadArchives()

    const ARCHIVE_STALE: ArchiveEntry = { ...ARCHIVE, name: 'stale-archive' }
    const ARCHIVE_FRESH: ArchiveEntry = { ...ARCHIVE, name: 'fresh-archive' }

    // Out-of-order resolution: the newer call's response lands first.
    resolveSecond({ data: [ARCHIVE_FRESH] })
    await secondCall
    expect(browser.archives.value).toEqual([ARCHIVE_FRESH])

    // The older call's response arrives after - it must not clobber the
    // fresher state already applied above.
    resolveFirst({ data: [ARCHIVE_STALE] })
    await firstCall
    expect(browser.archives.value).toEqual([ARCHIVE_FRESH])
  })

  it('clears archivesLoading once a superseded non-silent call finishes, even though a later silent call is now the latest', async () => {
    // A non-silent call (e.g. mount's own fetch) still in flight when a
    // background silent refresh (DataChanged/RepoOpChanged) starts becomes
    // "stale" by loadArchivesSeq - it must still clear archivesLoading when
    // it finishes, since the silent call that superseded it never touches
    // archivesLoading by design. Otherwise the flag would get stuck true
    // forever and the archive panel would never leave its loading state.
    let resolveNonSilent: (value: { data: ArchiveEntry[] }) => void = () => {}
    let resolveSilent: (value: { data: ArchiveEntry[] }) => void = () => {}
    vi.mocked(apiClient.get)
      .mockReturnValueOnce(
        new Promise((resolve) => {
          resolveNonSilent = resolve
        }),
      )
      .mockReturnValueOnce(
        new Promise((resolve) => {
          resolveSilent = resolve
        }),
      )

    const browser = useArchiveBrowser(ref(5))
    const nonSilentCall = browser.loadArchives()
    expect(browser.archivesLoading.value).toBe(true)

    const silentCall = browser.loadArchives(true)

    // The silent call resolves first and becomes the latest by seq - it
    // never touches archivesLoading either way.
    resolveSilent({ data: [ARCHIVE] })
    await silentCall
    expect(browser.archivesLoading.value).toBe(true)

    // The original non-silent call finishing must still clear the flag,
    // even though it's no longer the latest call.
    resolveNonSilent({ data: [ARCHIVE] })
    await nonSilentCall
    expect(browser.archivesLoading.value).toBe(false)
  })

  it('restoreEntry throws when the API reports failure', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { success: false, error_message: 'restore failed' },
    })

    const browser = useArchiveBrowser(ref(5))
    browser.selectedArchive.value = ARCHIVE
    const entry = { ...ROOT_ENTRY, type: '-', path: 'etc/nginx/nginx.conf' }

    await expect(browser.restoreEntry(entry)).rejects.toThrow('restore failed')
  })

  it('deleteArchiveByName throws when the API reports failure', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({
      data: { success: false, archive_name: ARCHIVE.name },
    })

    const browser = useArchiveBrowser(ref(5))

    await expect(browser.deleteArchiveByName(ARCHIVE)).rejects.toThrow('Archive delete failed')
  })

  it('deleteArchive returns false when the entry is not a directory', async () => {
    const browser = useArchiveBrowser(ref(5))
    browser.selectedArchive.value = ARCHIVE

    const result = await browser.deleteArchive({ ...ROOT_ENTRY, type: '-', path: '' })

    expect(result).toBe(false)
  })

  it('deleteArchive returns false when the entry path is not empty', async () => {
    const browser = useArchiveBrowser(ref(5))
    browser.selectedArchive.value = ARCHIVE

    const result = await browser.deleteArchive({ ...ROOT_ENTRY, type: 'd', path: 'not-empty' })

    expect(result).toBe(false)
  })
})
