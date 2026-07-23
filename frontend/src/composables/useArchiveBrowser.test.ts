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
})
